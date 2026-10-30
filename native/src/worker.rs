use crate::Options;
use crate::controller::{Audio, Controller};
use crate::journal::FileJournal;
use crate::latency::{Latency, Trigger};
use crate::measured::{MeasuredAudio, MeasuredJournal};
use crate::runtime::{self, HEARTBEAT_MS, Input, PROTOCOL, Reporter};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Command {
    Lease {
        active: bool,
        expires_at: u64,
        closed_generation: u64,
        capture_id: Option<String>,
        trigger: Option<Trigger>,
    },
    Shutdown,
}

fn serve<A: Audio>(mut audio: A, options: Options, backend: &str) -> Result<()> {
    let reporter = Reporter::new();
    if options.probe {
        let mut devices = Vec::new();
        for id in audio.active_devices()? {
            devices.push(json!({ "id": id, "muted": audio.get_mute(&id)? }));
        }
        reporter.finish(json!({ "type": "probe", "backend": backend, "devices": devices }));
        return Ok(());
    }
    let journal = FileJournal::open(options.directory.as_ref().unwrap())?;
    let mut controller = Controller::new(
        MeasuredAudio {
            inner: audio,
            reporter: reporter.clone(),
        },
        MeasuredJournal {
            inner: journal,
            reporter: reporter.clone(),
        },
        options.maximum_seconds * 1000,
    )?;
    let receiver = if options.recover_only {
        None
    } else {
        Some(runtime::input::<Command>())
    };
    let mut closing = options.recover_only.then(Instant::now);
    let mut last_status = None;
    let mut invalid = false;
    let mut sequence = 0_u64;
    let mut generation = 0;
    let mut heartbeat_at = Instant::now();

    controller.tick(runtime::now());
    if !options.recover_only {
        controller.prewarm();
    }
    let mut tick_at = Instant::now() + Duration::from_millis(100);
    let mut warm_at = Instant::now() + Duration::from_millis(500);
    let mut ready = false;
    loop {
        if Instant::now() >= tick_at {
            controller.tick(runtime::now());
            tick_at = Instant::now() + Duration::from_millis(100);
        }
        if closing.is_none() && Instant::now() >= warm_at {
            controller.prewarm();
            warm_at = Instant::now() + Duration::from_millis(500);
        }
        let status = controller.status();
        if !ready {
            ready =
                reporter.send(json!({ "type": "ready", "protocol": PROTOCOL, "backend": backend }));
        }
        if ready && last_status.as_ref() != Some(&status) {
            let mut message = serde_json::to_value(&status)?;
            message["type"] = json!("status");
            if reporter.send(message) {
                last_status = Some(status.clone());
            }
        }
        // Only the controller thread can attest that audio work has progressed.
        if ready && (sequence == 0 || heartbeat_at.elapsed() >= Duration::from_millis(HEARTBEAT_MS))
        {
            if reporter.send(json!({ "type": "heartbeat", "sequence": sequence })) {
                sequence += 1;
            }
            heartbeat_at = Instant::now();
        }
        if let Some(at) = closing {
            if ready
                && last_status.as_ref() == Some(&status)
                && (status.pending_restore == 0 || at.elapsed() >= Duration::from_secs(3))
            {
                reporter.finish(json!({
                    "type": "stopped", "restored": status.pending_restore == 0,
                    "pendingRestore": status.pending_restore
                }));
                if status.pending_restore > 0 {
                    bail!("Restoration is incomplete; recovery journal retained");
                }
                if invalid {
                    bail!("Invalid worker input; audio restored");
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        match receiver
            .as_ref()
            .unwrap()
            .recv_timeout(tick_at.saturating_duration_since(Instant::now()))
        {
            Ok(Input::Command(Command::Lease {
                active,
                expires_at,
                closed_generation,
                capture_id,
                trigger,
            })) => {
                let received_at = runtime::clock_us();
                let now = runtime::now();
                // Preserve a close/open edge even when the pipe writer coalesces updates.
                if closed_generation > generation {
                    controller.lease_capture(false, now, now, None);
                    generation = closed_generation;
                }
                if controller.lease_capture(active, now, expires_at, capture_id.as_deref()) {
                    // lease_capture already performed its initial pass; do not enumerate/read twice.
                    tick_at = Instant::now() + Duration::from_millis(100);
                    if let Some(trigger) = trigger
                        && let Some(latency) = Latency::measured(
                            trigger,
                            received_at,
                            controller.first_muted_at(),
                            controller.status().prepared.then(runtime::clock_us),
                        )
                    {
                        reporter.send(json!({ "type": "latency", "value": latency }));
                    }
                }
            }
            Ok(Input::Command(Command::Shutdown) | Input::Closed)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                controller.shutdown();
                closing = Some(Instant::now());
            }
            Ok(Input::Invalid) => {
                invalid = true;
                controller.shutdown();
                closing = Some(Instant::now());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

pub fn run(options: Options) -> Result<()> {
    if let Some(path) = options.simulation_file.clone() {
        return serve(
            crate::simulation::Simulation::new(path)?,
            options,
            "simulation",
        );
    }
    #[cfg(windows)]
    {
        let _guard = if options.probe {
            None
        } else {
            Some(crate::windows_audio::InstanceGuard::acquire()?)
        };
        serve(
            crate::windows_audio::WindowsAudio::new()?,
            options,
            "windows",
        )
    }
    #[cfg(not(windows))]
    bail!("The production audio backend requires Windows");
}
