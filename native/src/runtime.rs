use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

pub const PROTOCOL: u64 = 6;
pub const MAX_FRAME: usize = 16_384;
pub const HEARTBEAT_MS: u64 = 400;

#[cfg(windows)]
pub fn clock_us() -> u64 {
    use std::sync::OnceLock;
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    static FREQUENCY: OnceLock<i64> = OnceLock::new();
    let frequency = *FREQUENCY.get_or_init(|| {
        let mut value = 0;
        unsafe { QueryPerformanceFrequency(&mut value).expect("QPC is required on Windows") };
        value
    });
    let mut counter = 0;
    unsafe { QueryPerformanceCounter(&mut counter).expect("QPC is required on Windows") };
    (counter as u128 * 1_000_000 / frequency as u128) as u64
}

#[cfg(not(windows))]
pub fn clock_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[cfg(windows)]
pub fn now() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

#[cfg(not(windows))]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub enum Input<T> {
    Command(T),
    Invalid,
    Closed,
}

#[derive(Clone)]
pub struct Wake(SyncSender<()>);

impl Wake {
    pub fn channel() -> (Self, Receiver<()>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Self(sender), receiver)
    }

    pub fn signal(&self) {
        self.0.try_send(()).ok();
    }
}

pub fn input<T: DeserializeOwned + Send + 'static>() -> Receiver<Input<T>> {
    input_waking(None)
}

pub fn input_waking<T: DeserializeOwned + Send + 'static>(
    wake: Option<Wake>,
) -> Receiver<Input<T>> {
    let (sender, receiver) = mpsc::sync_channel(32);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        loop {
            let mut bytes = Vec::new();
            let message = match stdin.by_ref().take(4097).read_until(b'\n', &mut bytes) {
                Ok(0) => Input::Closed,
                Ok(n) if n <= 4096 && bytes.last() == Some(&b'\n') => {
                    serde_json::from_slice(&bytes)
                        .map(Input::Command)
                        .unwrap_or(Input::Invalid)
                }
                _ => Input::Invalid,
            };
            let done = !matches!(message, Input::Command(_));
            let sent = sender.send(message).is_ok();
            if let Some(wake) = &wake {
                wake.signal();
            }
            if !sent || done {
                return;
            }
        }
    });
    receiver
}

struct Message {
    value: Value,
    flushed: Option<SyncSender<()>>,
}

#[derive(Clone)]
pub struct Reporter(SyncSender<Message>);

impl Reporter {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Message>(32);
        std::thread::spawn(move || {
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            for message in receiver {
                if writeln!(stdout, "{}", message.value)
                    .and_then(|_| stdout.flush())
                    .is_err()
                {
                    return;
                }
                if let Some(ack) = message.flushed {
                    ack.try_send(()).ok();
                }
            }
        });
        Self(sender)
    }

    pub fn send(&self, value: Value) -> bool {
        self.0
            .try_send(Message {
                value,
                flushed: None,
            })
            .is_ok()
    }

    pub fn finish(&self, value: Value) {
        let (sender, receiver) = mpsc::sync_channel(1);
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut message = Message {
            value,
            flushed: Some(sender),
        };
        loop {
            match self.0.try_send(message) {
                Ok(()) => {
                    receiver
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .ok();
                    return;
                }
                Err(mpsc::TrySendError::Full(pending)) if Instant::now() < deadline => {
                    message = pending;
                    std::thread::sleep(Duration::from_millis(1));
                }
                _ => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_temporarily_full_output_queue_does_not_discard_the_final_receipt() {
        let (sender, receiver) = mpsc::sync_channel(1);
        assert!(
            sender
                .send(Message {
                    value: json!({ "type": "diagnostic" }),
                    flushed: None
                })
                .is_ok()
        );
        let reporter = Reporter(sender);
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            receiver.recv().unwrap();
            let final_message = receiver.recv_timeout(Duration::from_millis(300)).unwrap();
            assert_eq!(final_message.value["type"], "stopped");
            final_message.flushed.unwrap().send(()).unwrap();
        });
        reporter.finish(json!({ "type": "stopped" }));
        writer.join().unwrap();
    }
}
