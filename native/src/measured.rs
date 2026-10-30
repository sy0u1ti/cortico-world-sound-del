use crate::controller::Audio;
use crate::journal::Journal;
use crate::runtime::Reporter;
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeSet;
use std::time::Instant;

fn measured<T>(reporter: &Reporter, phase: &str, action: impl FnOnce() -> Result<T>) -> Result<T> {
    reporter.send(json!({ "type": "operation", "phase": phase, "done": false }));
    let at = Instant::now();
    let result = action();
    reporter.send(json!({
        "type": "operation", "phase": phase, "done": true,
        "ms": at.elapsed().as_secs_f64() * 1000.0, "ok": result.is_ok(),
    }));
    result
}

pub struct MeasuredAudio<A> {
    pub inner: A,
    pub reporter: Reporter,
}

impl<A: Audio> Audio for MeasuredAudio<A> {
    fn active_devices(&mut self) -> Result<Vec<String>> {
        measured(&self.reporter, "audio.enumerate", || {
            self.inner.active_devices()
        })
    }
    fn get_mute(&mut self, id: &str) -> Result<bool> {
        measured(&self.reporter, "audio.read", || self.inner.get_mute(id))
    }
    fn set_mute(&mut self, id: &str, value: bool) -> Result<()> {
        measured(
            &self.reporter,
            if value { "audio.mute" } else { "audio.restore" },
            || self.inner.set_mute(id, value),
        )
    }
    fn external_unmutes(&mut self) -> Result<Vec<String>> {
        self.inner.external_unmutes()
    }
}

pub struct MeasuredJournal<J> {
    pub inner: J,
    pub reporter: Reporter,
}

impl<J: Journal> Journal for MeasuredJournal<J> {
    fn load(&self) -> Result<BTreeSet<String>> {
        measured(&self.reporter, "journal.load", || self.inner.load())
    }
    fn save(&mut self, devices: &BTreeSet<String>) -> Result<()> {
        measured(&self.reporter, "journal.save", || self.inner.save(devices))
    }
}
