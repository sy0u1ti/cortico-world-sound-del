use crate::controller::Audio;
use crate::notifications::MuteObservation;
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, PROPERTYKEY,
};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole,
    IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
    eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateGuid, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};
use windows::core::{GUID, PCWSTR, implement, w};

const EVENT_CONTEXT: GUID = GUID::from_u128(0xb3de3882_e77e_4d7d_a846_8fcdb3701fd5);

pub struct InstanceGuard(HANDLE);

impl InstanceGuard {
    pub fn acquire() -> Result<Self> {
        Self::named(w!("Local\\CoopanionSoundDel.OutputMute.v1"))
    }

    pub fn supervisor() -> Result<Self> {
        Self::named(w!("Local\\CoopanionSoundDel.Supervisor.v2"))
    }

    fn named(name: PCWSTR) -> Result<Self> {
        let handle = unsafe { CreateMutexW(None, true, name)? };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            bail!(
                "Another Sound Del helper is running in this Windows session; disable its World first"
            );
        }
        Ok(Self(handle))
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    context: GUID,
    observation: Arc<MuteObservation>,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(
        &self,
        notification: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
    ) -> windows::core::Result<()> {
        if let Some(data) = unsafe { notification.as_ref() } {
            self.observation
                .notify(data.guidEventContext == self.context, data.bMuted.as_bool());
        }
        Ok(())
    }
}

struct Endpoint {
    volume: IAudioEndpointVolume,
    registration: Option<Registration>,
}

struct Registration {
    callback: IAudioEndpointVolumeCallback,
    observation: Arc<MuteObservation>,
    context: GUID,
}

impl Endpoint {
    fn unregister(&mut self) -> Result<()> {
        if let Some(registration) = &self.registration {
            registration.observation.close();
            unsafe {
                self.volume
                    .UnregisterControlChangeNotify(&registration.callback)?;
            }
        }
        self.registration = None;
        Ok(())
    }

    fn observe_acquisition(&mut self) -> Result<GUID> {
        self.unregister()?;
        let context = unsafe { CoCreateGuid()? };
        let observation = Arc::new(MuteObservation::default());
        let callback: IAudioEndpointVolumeCallback = VolumeCallback {
            context,
            observation: Arc::clone(&observation),
        }
        .into();
        unsafe { self.volume.RegisterControlChangeNotify(&callback)? };
        self.registration = Some(Registration {
            callback,
            observation,
            context,
        });
        Ok(context)
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

pub struct WindowsAudio {
    endpoints: BTreeMap<String, Endpoint>,
    enumerator: IMMDeviceEnumerator,
    topology_callback: IMMNotificationClient,
    topology_version: Arc<AtomicU64>,
    topology_read: Option<(u64, Instant)>,
    active_ids: Vec<String>,
    _apartment: Apartment,
}

#[implement(IMMNotificationClient)]
struct DeviceChanges(Arc<AtomicU64>);

impl IMMNotificationClient_Impl for DeviceChanges_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        self.0.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.0.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.0.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn OnDefaultDeviceChanged(
        &self,
        _: EDataFlow,
        _: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.0.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        self.0.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

impl Drop for WindowsAudio {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.topology_callback);
        }
    }
}

impl WindowsAudio {
    pub fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Cannot initialize audio COM apartment")?;
            let apartment = Apartment;
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let topology_version = Arc::new(AtomicU64::new(0));
            let topology_callback: IMMNotificationClient =
                DeviceChanges(Arc::clone(&topology_version)).into();
            enumerator.RegisterEndpointNotificationCallback(&topology_callback)?;
            Ok(Self {
                endpoints: BTreeMap::new(),
                enumerator,
                topology_callback,
                topology_version,
                topology_read: None,
                active_ids: Vec::new(),
                _apartment: apartment,
            })
        }
    }

    fn endpoint(&mut self, id: &str) -> Result<&mut Endpoint> {
        if !self.endpoints.contains_key(id) {
            let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
            let volume: IAudioEndpointVolume = unsafe {
                self.enumerator
                    .GetDevice(PCWSTR(wide.as_ptr()))?
                    .Activate(CLSCTX_ALL, None)?
            };
            self.endpoints.insert(
                id.into(),
                Endpoint {
                    volume,
                    registration: None,
                },
            );
        }
        Ok(self.endpoints.get_mut(id).unwrap())
    }
}

impl Audio for WindowsAudio {
    fn active_devices(&mut self) -> Result<Vec<String>> {
        let version = self.topology_version.load(Ordering::Acquire);
        // Device/default changes invalidate immediately; a periodic refresh covers omitted events.
        if self
            .topology_read
            .is_some_and(|(read, at)| read == version && at.elapsed() < Duration::from_secs(1))
        {
            return Ok(self.active_ids.clone());
        }
        let mut ids = Vec::new();
        unsafe {
            let devices = self
                .enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let count = devices.GetCount()?;
            if count > 128 {
                bail!("Too many playback devices");
            }
            for i in 0..count {
                let id = devices.Item(i)?.GetId()?;
                let text = id.to_string();
                CoTaskMemFree(Some(id.0.cast()));
                ids.push(text?);
            }
        }
        // Restore/mute the normal playback device first, without excluding other active outputs.
        if let Ok(device) = unsafe { self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            && let Ok(id) = unsafe { device.GetId() }
        {
            let default_id = unsafe { id.to_string() };
            unsafe { CoTaskMemFree(Some(id.0.cast())) };
            if let Ok(default_id) = default_id
                && let Some(index) = ids.iter().position(|id| *id == default_id)
            {
                ids.swap(0, index);
            }
        }
        self.endpoints.retain(|id, _| ids.contains(id));
        self.active_ids = ids.clone();
        self.topology_read = Some((version, Instant::now()));
        Ok(ids)
    }

    fn get_mute(&mut self, id: &str) -> Result<bool> {
        let result = unsafe {
            self.endpoint(id)?
                .volume
                .GetMute()
                .map(|value| value.as_bool())
        };
        if result.is_err() {
            self.endpoints.remove(id);
            self.topology_version.fetch_add(1, Ordering::Release);
        }
        result.context("Cannot read playback mute")
    }

    fn set_mute(&mut self, id: &str, muted: bool) -> Result<()> {
        let endpoint = self.endpoint(id)?;
        let context = if muted {
            endpoint.observe_acquisition()?
        } else if let Some(registration) = &endpoint.registration {
            registration.observation.close();
            registration.context
        } else {
            EVENT_CONTEXT
        };
        let result = unsafe { endpoint.volume.SetMute(muted, &context) };
        if result.is_err() {
            self.endpoints.remove(id);
            self.topology_version.fetch_add(1, Ordering::Release);
        }
        result.context("Cannot set playback mute")
    }

    fn external_unmutes(&mut self) -> Result<Vec<String>> {
        Ok(self
            .endpoints
            .iter()
            .filter_map(|(id, endpoint)| {
                endpoint
                    .registration
                    .as_ref()
                    .filter(|registration| registration.observation.overridden())
                    .map(|_| id.clone())
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_notifications_only_invalidate_the_cache_without_audio_work() {
        let version = Arc::new(AtomicU64::new(0));
        let callback: IMMNotificationClient = DeviceChanges(Arc::clone(&version)).into();
        unsafe {
            callback.OnDeviceAdded(PCWSTR::null()).unwrap();
            callback.OnDeviceRemoved(PCWSTR::null()).unwrap();
            callback
                .OnDeviceStateChanged(PCWSTR::null(), DEVICE_STATE_ACTIVE)
                .unwrap();
            callback
                .OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null())
                .unwrap();
            callback
                .OnPropertyValueChanged(PCWSTR::null(), PROPERTYKEY::default())
                .unwrap();
        }
        assert_eq!(version.load(Ordering::Acquire), 5);
    }

    #[test]
    fn callback_uses_acquisition_context_without_waiting_or_allocating() {
        let context = GUID::from_u128(1);
        let observation = Arc::new(MuteObservation::default());
        let callback: IAudioEndpointVolumeCallback = VolumeCallback {
            context,
            observation: Arc::clone(&observation),
        }
        .into();
        let mut event = AUDIO_VOLUME_NOTIFICATION_DATA::default();
        unsafe { callback.OnNotify(&mut event).unwrap() };
        assert!(!observation.overridden());
        event.guidEventContext = context;
        event.bMuted = true.into();
        unsafe { callback.OnNotify(&mut event).unwrap() };
        assert!(!observation.overridden());
        event.guidEventContext = GUID::zeroed();
        event.bMuted = false.into();
        unsafe { callback.OnNotify(&mut event).unwrap() };
        event.bMuted = true.into();
        unsafe { callback.OnNotify(&mut event).unwrap() };
        assert!(observation.overridden());
    }
}
