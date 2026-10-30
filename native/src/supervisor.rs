use crate::Options;
use crate::controller::{LEASE_TIMEOUT_MS, Status};
use crate::hotkey::{DirectHotkey, FALLBACK_POLL_MS, Hotkey};
use crate::latency::{Latency, Source, Trigger};
use crate::runtime::{self, HEARTBEAT_MS, Input, MAX_FRAME, PROTOCOL, Reporter, Wake};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROGRESS_TIMEOUT: Duration = Duration::from_millis(2_000);
const TERMINATION_TIMEOUT: Duration = Duration::from_millis(1_000);
const STOP_TIMEOUT: Duration = Duration::from_millis(6_000);

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum Command {
    Lease {
        active: bool,
        #[serde(rename = "captureId")]
        capture_id: Option<String>,
        hotkey: Option<Hotkey>,
    },
    SimulationKey {
        down: bool,
    },
    Shutdown,
}

#[derive(Clone, Default)]
struct LeaseState {
    expires_at: u64,
    closed_generation: u64,
    shutdown: bool,
    capture_id: Option<String>,
    trigger: Option<Trigger>,
}

struct Control {
    state: Mutex<LeaseState>,
    wake: SyncSender<()>,
}

impl Control {
    fn lease(&self, expires_at: u64, capture_id: Option<String>, trigger: Option<Trigger>) {
        let mut state = self.state.lock().unwrap();
        if state.shutdown || (state.expires_at == expires_at && state.capture_id == capture_id) {
            return;
        }
        if state.expires_at != 0 && (expires_at == 0 || state.capture_id != capture_id) {
            state.closed_generation += 1;
        }
        state.expires_at = expires_at;
        state.capture_id = capture_id;
        state.trigger = trigger;
        drop(state);
        self.wake.try_send(()).ok();
    }

    fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.expires_at = 0;
        state.capture_id = None;
        state.shutdown = true;
        drop(state);
        self.wake.try_send(()).ok();
    }
}

struct Worker {
    child: Child,
    control: Arc<Control>,
    frames: Receiver<Option<Value>>,
    last_progress: Instant,
    last_sequence: Option<u64>,
    ready: bool,
    stopped: Option<(bool, usize)>,
    status: Status,
    pipe_closed: bool,
    exited_at: Option<Instant>,
    failure: Option<String>,
    error: Option<String>,
    operation: Option<String>,
}

fn unknown_status() -> Status {
    Status {
        active: false,
        owned: 0,
        pending_restore: 0,
        overrides: 0,
        suspended: None,
        recovery_uncertain: true,
        capture_id: None,
        prepared: false,
        direct_hotkey: false,
    }
}

impl Worker {
    fn spawn(options: &Options, recovery: bool, supervisor_wake: Wake) -> Result<Self> {
        let mut command = ProcessCommand::new(std::env::current_exe()?);
        command
            .arg("--worker")
            .arg("--data-dir")
            .arg(options.directory.as_ref().unwrap())
            .arg("--max-mute-seconds")
            .arg(options.maximum_seconds.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(path) = &options.simulation_file {
            command.arg("--simulation-file").arg(path);
        }
        if recovery {
            command.arg("--recover-only");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().context("Cannot start audio worker")?;
        let mut input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (wake, wakeups) = mpsc::sync_channel(1);
        let control = Arc::new(Control {
            state: Mutex::new(LeaseState {
                shutdown: recovery,
                ..LeaseState::default()
            }),
            wake,
        });
        let writer_control = Arc::clone(&control);
        // A blocked pipe writer must never block the supervisor or extend a lease.
        std::thread::spawn(move || {
            if recovery {
                return;
            }
            loop {
                let state = writer_control.state.lock().unwrap().clone();
                let message = if state.shutdown {
                    json!({ "type": "shutdown" })
                } else {
                    json!({ "type": "lease", "active": state.expires_at != 0, "expiresAt": state.expires_at,
                        "closedGeneration": state.closed_generation, "captureId": state.capture_id,
                        "trigger": state.trigger })
                };
                if writeln!(input, "{message}")
                    .and_then(|_| input.flush())
                    .is_err()
                    || state.shutdown
                {
                    return;
                }
                wakeups
                    .recv_timeout(Duration::from_millis(HEARTBEAT_MS))
                    .ok();
            }
        });
        let (sender, frames) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let frame = match output
                    .by_ref()
                    .take((MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                {
                    Ok(0) => {
                        supervisor_wake.signal();
                        return;
                    }
                    Ok(n) if n <= MAX_FRAME && bytes.last() == Some(&b'\n') => {
                        serde_json::from_slice(&bytes).ok()
                    }
                    _ => None,
                };
                let invalid = frame.is_none();
                let sent = sender.send(frame).is_ok();
                supervisor_wake.signal();
                if !sent || invalid {
                    return;
                }
            }
        });
        Ok(Self {
            child,
            control,
            frames,
            last_progress: Instant::now(),
            last_sequence: None,
            ready: false,
            stopped: None,
            status: unknown_status(),
            pipe_closed: false,
            exited_at: None,
            failure: None,
            error: None,
            operation: None,
        })
    }

    fn frame(&mut self, frame: Value, backend: &str, reporter: &Reporter) -> Result<()> {
        match frame.get("type").and_then(Value::as_str) {
            Some("ready") if !self.ready => {
                if frame["protocol"] != PROTOCOL || frame["backend"] != backend {
                    bail!("Audio worker protocol or backend mismatch");
                }
                self.ready = true;
                self.last_progress = Instant::now();
            }
            Some("heartbeat") if self.ready => {
                let sequence = frame["sequence"]
                    .as_u64()
                    .context("Invalid worker heartbeat")?;
                if self.last_sequence.is_some_and(|last| sequence <= last) {
                    bail!("Audio worker heartbeat did not advance");
                }
                self.last_sequence = Some(sequence);
                self.last_progress = Instant::now();
            }
            Some("operation") => {
                let phase = frame["phase"].as_str().context("Missing operation phase")?;
                if !matches!(
                    phase,
                    "audio.enumerate"
                        | "audio.read"
                        | "audio.mute"
                        | "audio.restore"
                        | "journal.load"
                        | "journal.save"
                ) {
                    bail!("Invalid worker operation phase");
                }
                let done = frame["done"]
                    .as_bool()
                    .context("Invalid operation receipt")?;
                if done {
                    let ms = frame["ms"]
                        .as_f64()
                        .filter(|ms| ms.is_finite() && *ms >= 0.0 && *ms < 60_000.0)
                        .context("Invalid operation duration")?;
                    let ok = frame["ok"].as_bool().context("Invalid operation outcome")?;
                    // Completed calls prove progress; merely entering a call never renews a watchdog.
                    self.last_progress = Instant::now();
                    self.operation = None;
                    if ms >= 20.0 || !ok {
                        reporter.send(
                            json!({ "type": "diagnostic", "phase": phase, "ms": ms, "ok": ok }),
                        );
                    }
                } else {
                    self.operation = Some(phase.to_owned());
                }
            }
            Some("status") if self.ready => {
                let status: Status = serde_json::from_value(frame)?;
                if status.owned > 128
                    || status.pending_restore > 128
                    || status.overrides > 128
                    || status.suspended.as_ref().is_some_and(|s| s.len() > 4096)
                    || status
                        .capture_id
                        .as_ref()
                        .is_some_and(|s| !valid_capture_id(s))
                    || (status.prepared
                        && (!status.active
                            || status.capture_id.is_none()
                            || status.recovery_uncertain))
                {
                    bail!("Invalid audio worker status");
                }
                self.status = status;
            }
            Some("latency") if self.ready => {
                let latency: Latency = serde_json::from_value(frame["value"].clone())?;
                if !latency.valid() {
                    bail!("Invalid mute completion timing");
                }
                reporter.send(json!({ "type": "latency", "value": latency }));
            }
            Some("stopped") if self.ready && self.stopped.is_none() => {
                let restored = frame["restored"]
                    .as_bool()
                    .context("Invalid restoration acknowledgement")?;
                let pending = frame["pendingRestore"]
                    .as_u64()
                    .filter(|n| *n <= 128)
                    .context("Invalid pending restoration count")?
                    as usize;
                if restored
                    && (pending != 0
                        || self.status.active
                        || self.status.owned != 0
                        || self.status.pending_restore != 0
                        || self.status.recovery_uncertain)
                {
                    bail!("Inconsistent restoration acknowledgement");
                }
                self.stopped = Some((restored, pending));
            }
            Some("error") => {
                let message = frame["message"]
                    .as_str()
                    .filter(|s| s.len() <= 4096)
                    .context("Invalid worker error")?;
                self.error
                    .get_or_insert_with(|| format!("Audio worker failed: {message}"));
            }
            _ => bail!("Invalid audio worker output"),
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.control.shutdown();
    }
}

fn report_fault(reporter: &Reporter, fault: &mut Option<String>, message: String) {
    if fault.is_none() {
        reporter.send(json!({ "type": "error", "message": message }));
        *fault = Some(message);
    }
}

fn deliver(shortcut: &DirectHotkey, worker: &Worker) {
    if worker.ready && shortcut.fresh(runtime::now()) {
        let (deadline, id) = shortcut.intent();
        worker.control.lease(deadline, id, shortcut.trigger());
    }
}

pub fn run(options: Options) -> Result<()> {
    #[cfg(windows)]
    let _guard = if options.simulation_file.is_none() {
        Some(crate::windows_audio::InstanceGuard::supervisor()?)
    } else {
        None
    };
    let reporter = Reporter::new();
    let (wake, wakeups) = Wake::channel();
    let backend = if options.simulation_file.is_some() {
        "simulation"
    } else {
        "windows"
    };
    let receiver = if options.recover_only {
        None
    } else {
        Some(runtime::input_waking::<Command>(Some(wake.clone())))
    };
    let mut recovery = options.recover_only;
    let mut worker = Worker::spawn(&options, recovery, wake.clone())?;
    let mut closing = recovery.then(Instant::now);
    let mut terminating = None;
    let mut fault = None;
    let mut ready_reported = false;
    let mut last_status = None;
    let mut shortcut = DirectHotkey::default();
    let mut simulated_down = false;
    let mut sequence = 0_u64;
    let mut heartbeat_at = Instant::now();
    #[cfg(windows)]
    let mut monitor = if options.simulation_file.is_none() && !recovery {
        crate::key_input::Monitor::start(wake.clone()).ok()
    } else {
        None
    };

    loop {
        if closing.is_none() && fault.is_none() {
            shortcut.tick(runtime::now());
            #[cfg(windows)]
            if let Some(input) = &mut monitor {
                let overflow = input.overflowed();
                if !input.alive() || input.configure(shortcut.config(), overflow).is_err() {
                    monitor = None;
                    shortcut.baseline(shortcut.config().is_some_and(Hotkey::down), runtime::now());
                } else if overflow {
                    // Never turn a dropped release or partial tap sequence into a new recording.
                    shortcut.baseline(shortcut.config().is_some_and(Hotkey::down), runtime::now());
                } else {
                    for _ in 0..64 {
                        let Ok(event) = input.events.try_recv() else {
                            break;
                        };
                        if !input.accept(&event) {
                            continue;
                        }
                        if event.baseline || runtime::now().saturating_sub(event.at_ms) > 500 {
                            shortcut.baseline(event.down, runtime::now());
                        } else {
                            shortcut.sample(event.down, event.at_ms, event.trigger);
                        }
                        deliver(&shortcut, &worker);
                    }
                }
            }
        }
        if sequence == 0 || heartbeat_at.elapsed() >= Duration::from_millis(HEARTBEAT_MS) {
            if reporter
                .send(json!({ "type": "heartbeat", "sequence": sequence, "protocol": PROTOCOL }))
            {
                sequence += 1;
            }
            heartbeat_at = Instant::now();
        }
        if let Some(receiver) = &receiver {
            for _ in 0..32 {
                match receiver.try_recv() {
                    Ok(Input::Command(Command::Lease {
                        active,
                        capture_id: id,
                        hotkey,
                    })) if closing.is_none() && fault.is_none() => {
                        if (active && !id.as_deref().is_some_and(valid_capture_id))
                            || (!active && id.is_some())
                            || hotkey.as_ref().is_some_and(|config| !config.valid())
                        {
                            report_fault(&reporter, &mut fault, "Invalid capture lease".into());
                            closing = Some(Instant::now());
                            worker.control.shutdown();
                            break;
                        }
                        let changed = shortcut.config() != hotkey.as_ref();
                        shortcut.configure(
                            hotkey,
                            active,
                            id,
                            runtime::now().saturating_add(LEASE_TIMEOUT_MS),
                            runtime::clock_us(),
                        );
                        if changed {
                            #[cfg(windows)]
                            if let Some(input) = &mut monitor
                                && input.configure(shortcut.config(), false).is_err()
                            {
                                monitor = None;
                            }
                            #[cfg(windows)]
                            let hooked = monitor.is_some();
                            #[cfg(not(windows))]
                            let hooked = false;
                            if options.simulation_file.is_some() || !hooked {
                                let down = if options.simulation_file.is_some() {
                                    simulated_down
                                } else {
                                    shortcut.config().is_some_and(Hotkey::down)
                                };
                                shortcut.baseline(down, runtime::now());
                            }
                        }
                        deliver(&shortcut, &worker);
                    }
                    Ok(Input::Command(Command::Lease { .. })) => {}
                    Ok(Input::Command(Command::SimulationKey { down }))
                        if options.simulation_file.is_some() =>
                    {
                        simulated_down = down;
                        if closing.is_none() && fault.is_none() {
                            shortcut.tick(runtime::now());
                            shortcut.sample(down, runtime::now(), Trigger::now(Source::Simulation));
                            deliver(&shortcut, &worker);
                        }
                    }
                    Ok(Input::Command(Command::SimulationKey { .. })) => {
                        report_fault(
                            &reporter,
                            &mut fault,
                            "Simulation key input is unavailable on the Windows backend".into(),
                        );
                        closing = Some(Instant::now());
                        worker.control.shutdown();
                    }
                    Ok(Input::Command(Command::Shutdown) | Input::Closed)
                    | Err(mpsc::TryRecvError::Disconnected) => {
                        if closing.is_none() {
                            closing = Some(Instant::now());
                            worker.control.shutdown();
                        }
                        break;
                    }
                    Ok(Input::Invalid) => {
                        report_fault(
                            &reporter,
                            &mut fault,
                            "Invalid helper input; restoring audio".into(),
                        );
                        closing.get_or_insert_with(Instant::now);
                        worker.control.shutdown();
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        if closing.is_none() && fault.is_none() {
            #[cfg(windows)]
            let hooked = monitor.is_some();
            #[cfg(not(windows))]
            let hooked = false;
            if !hooked && options.simulation_file.is_none() {
                shortcut.sample(
                    shortcut.config().is_some_and(Hotkey::down),
                    runtime::now(),
                    Trigger::now(Source::Poll),
                );
            }
            shortcut.tick(runtime::now());
            // Preserve the expired deadline in the worker, including its safety latch.
            deliver(&shortcut, &worker);
        }
        let mut broken_frame = worker.failure.clone();
        for _ in 0..32 {
            match worker.frames.try_recv() {
                Ok(Some(frame)) => {
                    if let Err(error) = worker.frame(frame, backend, &reporter) {
                        broken_frame = Some(error.to_string());
                        break;
                    }
                }
                Ok(None) => {
                    broken_frame = Some("Invalid or oversized audio worker output".into());
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    worker.pipe_closed = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        worker.failure = broken_frame.clone();
        if let Some(error) = worker.error.clone() {
            report_fault(&reporter, &mut fault, error);
            closing.get_or_insert_with(Instant::now);
            worker.control.shutdown();
        }
        if worker.ready && !ready_reported && fault.is_none() && broken_frame.is_none() {
            ready_reported = reporter.send(json!({
                "type": "ready", "protocol": PROTOCOL, "backend": backend,
                "workerPid": worker.child.id()
            }));
            if closing.is_none() {
                deliver(&shortcut, &worker);
            }
        }
        if let Some(exit) = worker.child.try_wait()? {
            let at = worker.exited_at.get_or_insert_with(Instant::now);
            if !worker.pipe_closed && at.elapsed() < Duration::from_millis(500) {
                wakeups.recv_timeout(Duration::from_millis(20)).ok();
                continue;
            }
            if !worker.pipe_closed {
                broken_frame.get_or_insert_with(|| "Audio worker output did not close".into());
            }
            if let Some((restored, pending)) = worker.stopped
                && terminating.is_none()
                && broken_frame.is_none()
            {
                let expected = closing.is_some() || recovery;
                reporter.finish(json!({
                    "type": "stopped", "restored": restored, "pendingRestore": pending
                }));
                if restored && exit.success() && expected && fault.is_none() {
                    return Ok(());
                }
                bail!(
                    "{}",
                    fault.unwrap_or_else(|| if expected {
                        "Restoration incomplete; recovery journal retained".into()
                    } else {
                        "Audio worker exited unexpectedly; audio restoration was acknowledged"
                            .into()
                    })
                );
            }
            report_fault(
                &reporter,
                &mut fault,
                broken_frame.unwrap_or_else(|| {
                    "Audio worker exited without a verified restoration receipt".into()
                }),
            );
            if !recovery {
                // Confirm process death before a new owner can touch the recovery journal.
                recovery = true;
                terminating = None;
                closing = Some(Instant::now());
                worker.control.shutdown();
                worker = Worker::spawn(&options, true, wake.clone())?;
                continue;
            }
            reporter.finish(json!({ "type": "stopped", "restored": false, "pendingRestore": worker.status.owned.max(worker.status.pending_restore) }));
            bail!("Recovery worker failed; restoration remains unconfirmed");
        }
        let timed_out = worker.last_progress.elapsed() >= PROGRESS_TIMEOUT
            || closing.is_some_and(|at| at.elapsed() >= STOP_TIMEOUT);
        if terminating.is_none() && (timed_out || broken_frame.is_some()) {
            report_fault(
                &reporter,
                &mut fault,
                broken_frame.unwrap_or_else(|| {
                    format!(
                        "Audio worker unresponsive during {}; stopping it before bounded recovery",
                        worker
                            .operation
                            .as_deref()
                            .unwrap_or("startup or control loop"),
                    )
                }),
            );
            closing.get_or_insert_with(Instant::now);
            worker.control.shutdown();
            worker.child.kill().ok();
            terminating = Some(Instant::now());
        }
        if terminating.is_some_and(|at: Instant| at.elapsed() >= TERMINATION_TIMEOUT) {
            reporter.finish(json!({ "type": "stopped", "restored": false, "pendingRestore": worker.status.owned.max(worker.status.pending_restore) }));
            bail!("Worker termination unconfirmed; no competing recovery worker was started");
        }
        let mut status = worker.status.clone();
        status.direct_hotkey = closing.is_none() && fault.is_none() && shortcut.configured();
        #[cfg(windows)]
        if let Some(input) = &monitor {
            status.direct_hotkey &= input.armed();
        }
        if let Some(reason) = &fault {
            status.active = false;
            status.prepared = false;
            status.pending_restore = status.pending_restore.max(status.owned);
            status.recovery_uncertain = true;
            status.suspended = Some(reason.clone());
        }
        if last_status.as_ref() != Some(&status) {
            let mut frame = serde_json::to_value(&status)?;
            frame["type"] = json!("status");
            if reporter.send(frame) {
                last_status = Some(status);
            }
        }
        #[cfg(windows)]
        let hooked = monitor.is_some();
        #[cfg(not(windows))]
        let hooked = false;
        let wait_ms = if !hooked && options.simulation_file.is_none() && shortcut.configured() {
            FALLBACK_POLL_MS
        } else {
            50
        };
        // Input, host leases and worker receipts wake this wait immediately, including while idle.
        wakeups.recv_timeout(Duration::from_millis(wait_ms)).ok();
    }
}

fn valid_capture_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 96 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
