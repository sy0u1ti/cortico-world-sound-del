use crate::journal::Journal;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const LEASE_TIMEOUT_MS: u64 = 2500;

pub trait Audio {
    fn active_devices(&mut self) -> Result<Vec<String>>;
    fn get_mute(&mut self, id: &str) -> Result<bool>;
    fn set_mute(&mut self, id: &str, muted: bool) -> Result<()>;
    fn external_unmutes(&mut self) -> Result<Vec<String>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub active: bool,
    pub owned: usize,
    pub pending_restore: usize,
    pub overrides: usize,
    pub suspended: Option<String>,
    pub recovery_uncertain: bool,
    pub capture_id: Option<String>,
    pub prepared: bool,
    #[serde(default)]
    pub direct_hotkey: bool,
}

pub struct Controller<A: Audio, J: Journal> {
    pub audio: A,
    journal: J,
    saved: BTreeSet<String>,
    seen: BTreeSet<String>,
    relinquished: BTreeSet<String>,
    completed: BTreeSet<String>,
    active: bool,
    lease_until: u64,
    began_at: u64,
    maximum_ms: u64,
    suspended: Option<String>,
    capture_id: Option<String>,
    prepared: bool,
    first_muted_at: Option<u64>,
}

impl<A: Audio, J: Journal> Controller<A, J> {
    pub fn new(audio: A, journal: J, maximum_ms: u64) -> Result<Self> {
        let saved = journal.load()?;
        Ok(Self {
            audio,
            journal,
            saved,
            seen: BTreeSet::new(),
            relinquished: BTreeSet::new(),
            completed: BTreeSet::new(),
            active: false,
            lease_until: 0,
            began_at: 0,
            maximum_ms,
            suspended: None,
            capture_id: None,
            prepared: false,
            first_muted_at: None,
        })
    }

    pub fn status(&self) -> Status {
        Status {
            active: self.active,
            owned: self.saved.len(),
            pending_restore: if self.active { 0 } else { self.saved.len() },
            overrides: self.relinquished.len(),
            suspended: self.suspended.clone(),
            recovery_uncertain: false,
            capture_id: self.capture_id.clone(),
            prepared: self.active && self.prepared && self.suspended.is_none(),
            direct_hotkey: false,
        }
    }

    pub fn first_muted_at(&self) -> Option<u64> {
        self.first_muted_at
    }

    pub fn prewarm(&mut self) {
        if self.active || !self.saved.is_empty() || self.suspended.is_some() {
            return;
        }
        // Cache interfaces only. Never authorize recovery or reuse these mute values at capture time.
        if let Ok(devices) = self.audio.active_devices()
            && devices.len() <= 128
        {
            for id in devices {
                let _ = self.audio.get_mute(&id);
            }
        }
    }

    #[cfg(test)]
    pub fn lease(&mut self, active: bool, now: u64) {
        self.lease_with_deadline(active, now, now.saturating_add(LEASE_TIMEOUT_MS));
    }

    #[cfg(test)]
    pub fn lease_with_deadline(&mut self, active: bool, now: u64, expires_at: u64) {
        self.lease_capture(active, now, expires_at, active.then_some("test"));
    }

    pub fn lease_capture(
        &mut self,
        active: bool,
        now: u64,
        expires_at: u64,
        capture_id: Option<&str>,
    ) -> bool {
        if !active || self.capture_id.as_deref() != capture_id {
            self.prepared = false;
            if self.active {
                self.active = false;
                self.restore();
            }
        }
        if active && self.capture_id.as_deref() != capture_id {
            if !self.saved.is_empty() {
                self.suspend(anyhow::anyhow!("Previous capture recovery is pending"));
                return false;
            }
            self.seen.clear();
            self.relinquished.clear();
            self.completed.clear();
            self.suspended = None;
            self.first_muted_at = None;
        }
        self.capture_id = if active {
            capture_id.map(str::to_owned)
        } else {
            None
        };
        self.lease_until = expires_at.min(now.saturating_add(LEASE_TIMEOUT_MS));
        if active && self.lease_until <= now {
            self.active = false;
            self.suspended = Some("lease_expired".into());
            self.restore();
            return false;
        }
        if !active {
            self.active = false;
            self.restore();
            if self.saved.is_empty() {
                self.suspended = None;
                self.seen.clear();
                self.relinquished.clear();
                self.completed.clear();
            }
            return false;
        }
        if self.active || self.suspended.is_some() {
            return false;
        }
        if !self.saved.is_empty() {
            self.suspended = Some("recovery_pending".into());
            return false;
        }
        // Old notifications belong to earlier utterances.
        if let Err(error) = self.audio.external_unmutes() {
            self.suspend(error);
            return false;
        }
        self.active = true;
        self.began_at = now;
        self.tick(now);
        true
    }

    pub fn tick(&mut self, now: u64) {
        self.prepared = false;
        if self.active && now >= self.lease_until {
            self.active = false;
            self.suspended = Some("lease_expired".into());
        }
        if self.active && now.saturating_sub(self.began_at) >= self.maximum_ms {
            self.active = false;
            self.suspended = Some("maximum_duration".into());
        }
        let readbacks = match self.observe_overrides() {
            Ok(readbacks) => readbacks,
            Err(error) => {
                self.suspend(error);
                BTreeMap::new()
            }
        };
        if self.active {
            match self.mute_new_devices(&readbacks) {
                Ok(prepared) => self.prepared = prepared,
                Err(error) => self.suspend(error),
            }
        }
        if !self.active {
            self.restore();
        }
    }

    pub fn shutdown(&mut self) {
        self.active = false;
        self.restore();
    }

    fn suspend(&mut self, error: anyhow::Error) {
        self.active = false;
        self.suspended = Some(format!("audio_or_journal_error: {error}"));
    }

    fn observe_overrides(&mut self) -> Result<BTreeMap<String, bool>> {
        let mut readbacks = BTreeMap::new();
        for id in self.audio.external_unmutes()? {
            if self.saved.contains(&id) {
                self.relinquished.insert(id);
            }
        }
        // Readback also covers drivers that omit change notifications.
        for id in self.saved.clone() {
            if self.active
                && let Ok(muted) = self.audio.get_mute(&id)
            {
                if !muted {
                    self.relinquished.insert(id.clone());
                }
                readbacks.insert(id, muted);
            }
        }
        self.completed
            .extend(self.saved.intersection(&self.relinquished).cloned());
        self.commit_completed()?;
        Ok(readbacks)
    }

    fn mute_new_devices(&mut self, readbacks: &BTreeMap<String, bool>) -> Result<bool> {
        let devices = self.audio.active_devices()?;
        if devices.len() > 128 {
            bail!("Too many playback devices");
        }
        let mut prepared = !devices.is_empty();
        let mut to_mute = Vec::new();
        let mut next = self.saved.clone();
        for id in devices {
            if self.relinquished.contains(&id) {
                prepared = false;
                continue;
            }
            if self.seen.contains(&id) {
                prepared &= match readbacks.get(&id) {
                    Some(muted) => *muted,
                    None => self.audio.get_mute(&id)?,
                };
                continue;
            }
            self.seen.insert(id.clone());
            if self.audio.get_mute(&id)? {
                continue;
            }
            next.insert(id.clone());
            if next.len() > 128 || id.is_empty() || id.len() > 4096 {
                bail!("Playback device recovery limit exceeded");
            }
            to_mute.push(id);
        }
        // One durable write authorizes the whole batch, before the first device is changed.
        if next != self.saved {
            self.journal.save(&next)?;
            self.saved = next;
        }
        for id in to_mute {
            self.audio.set_mute(&id, true)?;
            if !self.audio.get_mute(&id)? {
                bail!("Mute readback did not match");
            }
            self.first_muted_at
                .get_or_insert_with(crate::runtime::clock_us);
        }
        Ok(prepared)
    }

    fn commit_completed(&mut self) -> Result<()> {
        let next = self.saved.difference(&self.completed).cloned().collect();
        if next != self.saved {
            self.journal.save(&next)?;
            self.saved = next;
        }
        self.completed.retain(|id| self.saved.contains(id));
        Ok(())
    }

    fn restore(&mut self) {
        // Process overrides before restoration, including a manual unmute followed by a mute.
        if let Err(error) = self.observe_overrides() {
            self.suspended = Some(format!("audio_or_journal_error: {error}"));
        }
        for id in self.saved.clone() {
            if self.completed.contains(&id) {
                continue;
            }
            let result = (|| -> Result<()> {
                if !self.relinquished.contains(&id) && self.audio.get_mute(&id)? {
                    self.audio.set_mute(&id, false)?;
                    if self.audio.get_mute(&id)? {
                        bail!("Restore readback did not match");
                    }
                }
                self.completed.insert(id.clone());
                Ok(())
            })();
            if let Err(error) = result {
                self.suspended = Some(format!("restore_pending: {error}"));
            }
        }
        if let Err(error) = self.commit_completed() {
            self.suspended = Some(format!("restore_pending: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Device {
        muted: bool,
        active: bool,
        fail_set: bool,
        fail_get: bool,
    }

    #[derive(Default)]
    struct FakeAudio {
        devices: BTreeMap<String, Device>,
        unmutes: Vec<String>,
        reads: usize,
        sets: usize,
    }

    impl FakeAudio {
        fn add(&mut self, id: &str, muted: bool) {
            self.devices.insert(
                id.into(),
                Device {
                    muted,
                    active: true,
                    ..Device::default()
                },
            );
        }
    }
    impl Audio for FakeAudio {
        fn active_devices(&mut self) -> Result<Vec<String>> {
            Ok(self
                .devices
                .iter()
                .filter(|(_, d)| d.active)
                .map(|(id, _)| id.clone())
                .collect())
        }
        fn get_mute(&mut self, id: &str) -> Result<bool> {
            self.reads += 1;
            let device = &self.devices[id];
            if !device.active || device.fail_get {
                bail!("Device unavailable");
            }
            Ok(device.muted)
        }
        fn set_mute(&mut self, id: &str, muted: bool) -> Result<()> {
            self.sets += 1;
            let device = self.devices.get_mut(id).unwrap();
            if device.fail_set {
                bail!("Set failed");
            }
            device.muted = muted;
            Ok(())
        }
        fn external_unmutes(&mut self) -> Result<Vec<String>> {
            Ok(std::mem::take(&mut self.unmutes))
        }
    }

    #[derive(Default)]
    struct MemoryJournal {
        saved: BTreeSet<String>,
        fail: bool,
        writes: usize,
    }
    impl Journal for MemoryJournal {
        fn load(&self) -> Result<BTreeSet<String>> {
            Ok(self.saved.clone())
        }
        fn save(&mut self, devices: &BTreeSet<String>) -> Result<()> {
            if self.fail {
                bail!("Journal unavailable");
            }
            self.saved = devices.clone();
            self.writes += 1;
            Ok(())
        }
    }
    fn setup() -> Controller<FakeAudio, MemoryJournal> {
        let mut audio = FakeAudio::default();
        audio.add("speaker", false);
        audio.add("already-muted", true);
        Controller::new(audio, MemoryJournal::default(), 10_000).unwrap()
    }

    #[test]
    fn ordinary_cycle_preserves_preexisting_mute() {
        let mut c = setup();
        c.lease(true, 0);
        assert!(c.audio.devices["speaker"].muted);
        assert_eq!(c.journal.saved, BTreeSet::from(["speaker".into()]));
        c.lease(false, 500);
        assert!(!c.audio.devices["speaker"].muted);
        assert!(c.audio.devices["already-muted"].muted);
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn prewarm_never_mutes_journals_or_reuses_old_mute_state() {
        let mut c = setup();
        c.prewarm();
        assert_eq!(c.audio.reads, 2);
        assert_eq!(c.audio.sets, 0);
        assert_eq!(c.journal.writes, 0);
        assert!(c.saved.is_empty());
        c.audio.devices.get_mut("speaker").unwrap().muted = true;
        c.lease(true, 10);
        assert!(c.status().prepared);
        assert!(c.saved.is_empty());
        assert!(c.first_muted_at().is_none());
        c.shutdown();
        assert!(c.audio.devices["speaker"].muted);
    }

    #[test]
    fn periodic_verification_reads_owned_devices_once_not_twice() {
        let mut c = setup();
        c.lease(true, 0);
        let before = c.audio.reads;
        c.tick(100);
        assert_eq!(c.audio.reads - before, 2);
        assert!(c.status().prepared);
        assert!(c.first_muted_at().is_some());
    }

    #[test]
    fn duplicate_active_leases_do_not_overwrite_original_state() {
        let mut c = setup();
        c.lease(true, 0);
        c.lease(true, 400);
        c.tick(800);
        c.lease(false, 900);
        assert!(!c.audio.devices["speaker"].muted);
    }

    #[test]
    fn lost_lease_restores_and_latches_until_a_closed_gate() {
        let mut c = setup();
        c.lease(true, 0);
        c.tick(LEASE_TIMEOUT_MS);
        assert!(!c.audio.devices["speaker"].muted);
        c.lease(true, 3000);
        assert!(!c.audio.devices["speaker"].muted);
        c.lease(false, 3100);
        c.lease(true, 3200);
        assert!(c.audio.devices["speaker"].muted);
    }

    #[test]
    fn maximum_duration_is_not_extended_by_heartbeats() {
        let mut c = setup();
        c.lease(true, 0);
        for now in (1000..=10000).step_by(1000) {
            c.lease(true, now);
            c.tick(now);
        }
        assert!(!c.audio.devices["speaker"].muted);
        assert_eq!(c.status().suspended.as_deref(), Some("maximum_duration"));
        c.lease(true, 11000);
        assert!(!c.audio.devices["speaker"].muted);
    }

    #[test]
    fn new_device_is_muted_and_both_are_restored() {
        let mut c = setup();
        c.lease(true, 0);
        c.audio.add("headphone", false);
        c.tick(500);
        assert!(c.audio.devices["headphone"].muted);
        c.shutdown();
        assert!(!c.audio.devices["headphone"].muted);
        assert!(!c.audio.devices["speaker"].muted);
    }

    #[test]
    fn disconnected_device_keeps_recovery_record_until_reconnected() {
        let mut c = setup();
        c.lease(true, 0);
        c.audio.devices.get_mut("speaker").unwrap().active = false;
        c.shutdown();
        assert_eq!(c.status().pending_restore, 1);
        c.audio.devices.get_mut("speaker").unwrap().active = true;
        c.tick(2000);
        assert!(!c.audio.devices["speaker"].muted);
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn manual_unmute_is_not_fought_or_remuted() {
        let mut c = setup();
        c.lease(true, 0);
        c.audio.devices.get_mut("speaker").unwrap().muted = false;
        c.tick(500);
        assert!(!c.audio.devices["speaker"].muted);
        assert_eq!(c.status().overrides, 1);
        c.tick(1000);
        assert!(!c.audio.devices["speaker"].muted);
        c.shutdown();
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn manual_unmute_then_remute_is_preserved_at_end() {
        let mut c = setup();
        c.lease(true, 0);
        c.audio.unmutes.push("speaker".into());
        c.shutdown();
        assert!(c.audio.devices["speaker"].muted);
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn journal_failure_prevents_a_new_mute() {
        let mut c = setup();
        c.journal.fail = true;
        c.lease(true, 0);
        assert!(!c.audio.devices["speaker"].muted);
        assert!(!c.status().active);
        assert!(c.status().suspended.is_some());
    }

    #[test]
    fn partial_apply_failure_rolls_back_successful_devices() {
        let mut c = setup();
        c.audio.add("z-failed", false);
        c.audio.devices.get_mut("z-failed").unwrap().fail_set = true;
        c.lease(true, 0);
        assert!(!c.audio.devices["speaker"].muted);
        assert!(!c.status().active);
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn restore_failure_retains_journal_for_retry() {
        let mut c = setup();
        c.lease(true, 0);
        c.audio.devices.get_mut("speaker").unwrap().fail_set = true;
        c.shutdown();
        assert_eq!(c.status().pending_restore, 1);
        c.audio.devices.get_mut("speaker").unwrap().fail_set = false;
        c.tick(500);
        assert!(!c.audio.devices["speaker"].muted);
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn journal_failure_does_not_discard_later_manual_override_notifications() {
        let mut c = setup();
        c.audio.add("headphone", false);
        c.lease(true, 0);
        c.audio.unmutes = vec!["headphone".into(), "speaker".into()];
        c.journal.fail = true;
        c.shutdown();
        assert!(c.audio.devices["speaker"].muted);
        assert!(c.audio.devices["headphone"].muted);
        c.journal.fail = false;
        c.tick(500);
        assert!(c.audio.devices["speaker"].muted);
        assert!(c.audio.devices["headphone"].muted);
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn restart_recovers_only_recorded_devices() {
        let mut first = setup();
        first.lease(true, 0);
        let mut restarted = Controller::new(first.audio, first.journal, 10_000).unwrap();
        restarted.tick(0);
        assert!(!restarted.audio.devices["speaker"].muted);
        assert!(restarted.audio.devices["already-muted"].muted);
        assert!(restarted.journal.saved.is_empty());
    }

    #[test]
    fn queued_expired_lease_cannot_restart_muting() {
        let mut c = setup();
        c.lease(true, 0);
        c.lease_with_deadline(true, 5_000, 2_500);
        assert!(!c.audio.devices["speaker"].muted);
        assert_eq!(c.status().suspended.as_deref(), Some("lease_expired"));
        c.lease_with_deadline(true, 5_100, 7_500);
        assert!(!c.audio.devices["speaker"].muted);
    }

    #[test]
    fn pre_mute_volume_notification_does_not_remove_recovery() {
        use crate::notifications::MuteObservation;
        struct NotifyingAudio {
            muted: bool,
            observation: MuteObservation,
        }
        impl Audio for NotifyingAudio {
            fn active_devices(&mut self) -> Result<Vec<String>> {
                Ok(vec!["speaker".into()])
            }
            fn get_mute(&mut self, _: &str) -> Result<bool> {
                Ok(self.muted)
            }
            fn set_mute(&mut self, _: &str, value: bool) -> Result<()> {
                if value {
                    self.observation.notify(false, false);
                    self.observation.notify(true, true);
                } else {
                    self.observation.close();
                }
                self.muted = value;
                Ok(())
            }
            fn external_unmutes(&mut self) -> Result<Vec<String>> {
                Ok(if self.observation.overridden() {
                    vec!["speaker".into()]
                } else {
                    vec![]
                })
            }
        }
        let audio = NotifyingAudio {
            muted: false,
            observation: MuteObservation::default(),
        };
        let mut c = Controller::new(audio, MemoryJournal::default(), 10_000).unwrap();
        c.lease(true, 0);
        c.tick(100);
        assert_eq!(c.status().owned, 1);
        c.lease(false, 200);
        assert!(!c.audio.muted);
        assert!(c.journal.saved.is_empty());
    }

    #[test]
    fn preparation_is_bound_to_the_capture_and_all_active_outputs() {
        let mut c = setup();
        c.lease_capture(true, 0, 2500, Some("first"));
        assert!(c.status().prepared);
        assert_eq!(c.status().capture_id.as_deref(), Some("first"));
        c.lease_capture(true, 100, 2600, Some("second"));
        assert!(c.status().prepared);
        assert_eq!(c.status().capture_id.as_deref(), Some("second"));
        c.audio.devices.get_mut("already-muted").unwrap().muted = false;
        c.tick(150);
        assert!(!c.status().prepared);
        c.lease_capture(false, 200, 200, None);
        assert!(!c.status().prepared);
        assert!(!c.audio.devices["speaker"].muted);
    }

    #[test]
    fn no_output_devices_is_not_a_successful_preparation() {
        let mut c =
            Controller::new(FakeAudio::default(), MemoryJournal::default(), 10_000).unwrap();
        c.lease_capture(true, 0, 2500, Some("empty"));
        assert!(c.status().active);
        assert!(!c.status().prepared);
    }

    #[test]
    fn a_three_device_cycle_uses_one_write_before_mute_and_one_after_restore() {
        let mut c = setup();
        c.audio.add("headphones", false);
        c.audio.add("other-output", false);
        c.lease(true, 0);
        assert_eq!(c.journal.writes, 1);
        assert_eq!(c.journal.saved.len(), 3);
        assert!(c.audio.devices["headphones"].muted);
        assert!(c.audio.devices["other-output"].muted);
        assert!(c.audio.devices["speaker"].muted);
        c.lease(false, 100);
        assert_eq!(c.journal.writes, 2);
        assert!(c.journal.saved.is_empty());
        assert!(!c.audio.devices["speaker"].muted);
        assert!(c.audio.devices["already-muted"].muted);
    }

    #[test]
    fn failed_completion_write_does_not_restore_the_same_device_twice() {
        let mut c = setup();
        c.lease(true, 0);
        c.journal.fail = true;
        c.shutdown();
        assert!(!c.audio.devices["speaker"].muted);
        assert_eq!(c.status().pending_restore, 1);
        c.audio.devices.get_mut("speaker").unwrap().muted = true;
        c.journal.fail = false;
        c.tick(200);
        assert!(
            c.audio.devices["speaker"].muted,
            "Do not undo a later manual mute after restoring successfully"
        );
        assert!(c.journal.saved.is_empty());
    }
}
