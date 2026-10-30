use crate::controller::Audio;
use crate::journal::atomic_json;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SimDevice {
    pub muted: bool,
    pub active: bool,
    #[serde(default)]
    pub fail_set: bool,
    #[serde(default)]
    pub fail_get: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SimState {
    pub devices: BTreeMap<String, SimDevice>,
    #[serde(default)]
    pub external_unmutes: Vec<String>,
    #[serde(default)]
    pub block_operation: Option<String>,
    #[serde(default)]
    pub mute_delay_ms: u64,
}

pub struct Simulation {
    path: PathBuf,
}
impl Simulation {
    pub fn new(path: PathBuf) -> Result<Self> {
        if !path.is_absolute() {
            bail!("The simulation file must be absolute");
        }
        let backend = Self { path };
        backend.read()?;
        Ok(backend)
    }
    fn read(&self) -> Result<SimState> {
        let bytes = fs::read(&self.path)?;
        if bytes.len() > 1_048_576 {
            bail!("Simulation file exceeds size limit");
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn maybe_block(&self, state: &SimState, operation: &str) -> Result<()> {
        if state.block_operation.as_deref() == Some(operation) {
            fs::write(
                self.path.with_extension("blocked"),
                std::process::id().to_string(),
            )?;
            loop {
                std::thread::park();
            }
        }
        Ok(())
    }
}

impl Audio for Simulation {
    fn active_devices(&mut self) -> Result<Vec<String>> {
        let state = self.read()?;
        self.maybe_block(&state, "enumerate")?;
        Ok(state
            .devices
            .into_iter()
            .filter(|(_, d)| d.active)
            .map(|(id, _)| id)
            .collect())
    }
    fn get_mute(&mut self, id: &str) -> Result<bool> {
        let state = self.read()?;
        self.maybe_block(&state, "get")?;
        let device = state
            .devices
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown simulated device"))?;
        if !device.active || device.fail_get {
            bail!("Simulated device unavailable");
        }
        Ok(device.muted)
    }
    fn set_mute(&mut self, id: &str, muted: bool) -> Result<()> {
        let mut state = self.read()?;
        self.maybe_block(&state, if muted { "mute" } else { "restore" })?;
        if muted && state.mute_delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(
                state.mute_delay_ms.min(5_000),
            ));
        }
        let device = state
            .devices
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown simulated device"))?;
        if !device.active || device.fail_set {
            bail!("Simulated mute failure");
        }
        device.muted = muted;
        atomic_json(&self.path, &state)
    }
    fn external_unmutes(&mut self) -> Result<Vec<String>> {
        let mut state = self.read()?;
        let events = std::mem::take(&mut state.external_unmutes);
        if !events.is_empty() {
            atomic_json(&self.path, &state)?;
        }
        Ok(events)
    }
}
