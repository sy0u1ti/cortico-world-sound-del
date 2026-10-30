use crate::hotkey::Hotkey;

#[derive(Clone, Default, PartialEq, Eq)]
struct Chord {
    keys: Vec<u16>,
    pressed: [u64; 4],
}

fn sides(key: u16) -> [u16; 2] {
    match key {
        0x10 => [0xa0, 0xa1],
        0x11 => [0xa2, 0xa3],
        0x12 => [0xa4, 0xa5],
        _ => [key, key],
    }
}

fn normalize(key: u16, scan: u32, extended: bool) -> u16 {
    match key {
        0x10 => {
            if scan == 0x36 {
                0xa1
            } else {
                0xa0
            }
        }
        0x11 => {
            if extended {
                0xa3
            } else {
                0xa2
            }
        }
        0x12 => {
            if extended {
                0xa5
            } else {
                0xa4
            }
        }
        _ => key,
    }
}

impl Chord {
    fn snapshot(config: &Hotkey, mut down: impl FnMut(u16) -> bool) -> Self {
        let mut chord = Self {
            keys: config.keys.clone(),
            ..Self::default()
        };
        for key in &config.keys {
            for side in sides(*key) {
                chord.set(side, down(side));
            }
        }
        chord
    }

    fn set(&mut self, key: u16, down: bool) {
        if key >= 256 {
            return;
        }
        let word = usize::from(key / 64);
        let bit = 1_u64 << (key % 64);
        if down {
            self.pressed[word] |= bit;
        } else {
            self.pressed[word] &= !bit;
        }
    }

    fn down(&self) -> bool {
        !self.keys.is_empty()
            && self.keys.iter().all(|key| {
                sides(*key)
                    .iter()
                    .any(|side| self.pressed[usize::from(side / 64)] & (1_u64 << (side % 64)) != 0)
            })
    }

    fn event(&mut self, key: u16, down: bool) -> Option<bool> {
        if !self.keys.iter().any(|wanted| sides(*wanted).contains(&key)) {
            return None;
        }
        let before = self.down();
        self.set(key, down);
        let after = self.down();
        (before != after).then_some(after)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::hotkey::FALLBACK_POLL_MS;
    use crate::latency::{Source, Trigger};
    use crate::runtime::{self, Wake};
    use anyhow::{Context, Result, bail};
    use std::cell::RefCell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, SyncSender};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{LPARAM, LRESULT, WAIT_FAILED, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const AUDIT_MS: u64 = 50;
    const RESUME_MS: u64 = 500;
    const WAKE_MESSAGE: u32 = WM_APP + 42;

    fn discard_pending(events: &Receiver<KeyEvent>) {
        for _ in 0..64 {
            if events.try_recv().is_err() {
                break;
            }
        }
    }

    #[derive(Clone, Copy)]
    pub struct KeyEvent {
        pub epoch: u64,
        pub down: bool,
        pub baseline: bool,
        pub at_ms: u64,
        pub trigger: Trigger,
    }

    struct Configuration {
        epoch: u64,
        hotkey: Option<Hotkey>,
    }

    struct Listener {
        config: Option<Hotkey>,
        chord: Chord,
        epoch: u64,
        last_edge: Instant,
        mismatch: Option<[u64; 4]>,
        output: SyncSender<KeyEvent>,
        overflow: Arc<AtomicBool>,
        wake: Wake,
    }

    impl Listener {
        fn key(&mut self, key: u16, down: bool) {
            if !self.mouse()
                && let Some(down) = self.chord.event(key, down)
            {
                self.last_edge = Instant::now();
                self.mismatch = None;
                self.emit(down, false, Source::Hook);
            }
        }

        fn emit(&self, down: bool, baseline: bool, source: Source) {
            let event = KeyEvent {
                epoch: self.epoch,
                down,
                baseline,
                at_ms: runtime::now(),
                trigger: Trigger::now(source),
            };
            if self.output.try_send(event).is_err() {
                self.overflow.store(true, Ordering::Release);
            }
            self.wake.signal();
        }

        fn snapshot(&self) -> Chord {
            self.config.as_ref().map_or_else(Chord::default, |config| {
                Chord::snapshot(config, |key| unsafe {
                    GetAsyncKeyState(i32::from(key)) < 0
                })
            })
        }

        fn baseline(&mut self) {
            self.chord = self.snapshot();
            self.mismatch = None;
            self.last_edge = Instant::now();
            self.emit(self.chord.down(), true, Source::Poll);
        }

        fn mouse(&self) -> bool {
            self.config
                .as_ref()
                .is_some_and(|key| key.keys.iter().any(|k| (4..=6).contains(k)))
        }
    }

    thread_local! {
        static LISTENER: RefCell<Option<Listener>> = const { RefCell::new(None) };
    }

    unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32
            && matches!(
                wparam.0 as u32,
                WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
            )
        {
            // No audio, disk, waiting, key injection or unrelated-key storage on this callback.
            if let Some(data) = unsafe { (lparam.0 as *const KBDLLHOOKSTRUCT).as_ref() } {
                let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                let key = normalize(
                    data.vkCode as u16,
                    data.scanCode,
                    data.flags.contains(LLKHF_EXTENDED),
                );
                let _ = LISTENER.try_with(|slot| {
                    if let Ok(mut slot) = slot.try_borrow_mut()
                        && let Some(listener) = slot.as_mut()
                    {
                        listener.key(key, down);
                    }
                });
            }
        }
        // Always forward, including irrelevant keys, repeats and injected events.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    struct Hook(HHOOK);
    impl Hook {
        fn install() -> Result<Self> {
            let module = unsafe { GetModuleHandleW(None)? };
            Ok(Self(unsafe {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(module.into()), 0)?
            }))
        }
    }
    impl Drop for Hook {
        fn drop(&mut self) {
            unsafe {
                let _ = UnhookWindowsHookEx(self.0);
            }
        }
    }

    struct Alive(Arc<AtomicBool>, Wake);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
            self.1.signal();
        }
    }

    pub struct Monitor {
        commands: SyncSender<Configuration>,
        pub events: Receiver<KeyEvent>,
        epoch: u64,
        config: Option<Hotkey>,
        thread_id: u32,
        alive: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        overflow: Arc<AtomicBool>,
        armed: bool,
    }

    impl Monitor {
        pub fn start(wake: Wake) -> Result<Self> {
            let (commands, incoming) = mpsc::sync_channel::<Configuration>(8);
            let (output, events) = mpsc::sync_channel(64);
            let (started, ready) = mpsc::sync_channel(1);
            let alive = Arc::new(AtomicBool::new(true));
            let stop = Arc::new(AtomicBool::new(false));
            let overflow = Arc::new(AtomicBool::new(false));
            let guard = Alive(Arc::clone(&alive), wake.clone());
            let thread_stop = Arc::clone(&stop);
            let listener = Listener {
                config: None,
                chord: Chord::default(),
                epoch: 0,
                last_edge: Instant::now(),
                mismatch: None,
                output,
                overflow: Arc::clone(&overflow),
                wake,
            };
            std::thread::Builder::new()
                .name("shortcut-events".into())
                .spawn(move || {
                    let _guard = guard;
                    runtime::clock_us();
                    let mut message = MSG::default();
                    unsafe {
                        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                    }
                    LISTENER.with(|slot| *slot.borrow_mut() = Some(listener));
                    let mut hook = Hook::install().ok();
                    if started.send(unsafe { GetCurrentThreadId() }).is_err() {
                        return;
                    }
                    let mut audit_at = Instant::now();
                    let mut loop_at = Instant::now();
                    while !thread_stop.load(Ordering::Acquire) {
                        for config in incoming.try_iter() {
                            LISTENER.with(|slot| {
                                let mut slot = slot.borrow_mut();
                                let listener = slot.as_mut().unwrap();
                                listener.config = config.hotkey;
                                listener.epoch = config.epoch;
                                listener.baseline();
                            });
                        }
                        // PeekMessage dispatches low-level hook calls on their installing thread.
                        for _ in 0..64 {
                            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }
                                .as_bool()
                            {
                                break;
                            }
                            if message.message == WM_QUIT {
                                thread_stop.store(true, Ordering::Release);
                            }
                        }
                        let resumed = loop_at.elapsed() >= Duration::from_millis(RESUME_MS);
                        loop_at = Instant::now();
                        let poll = hook.is_none()
                            || LISTENER.with(|slot| slot.borrow().as_ref().unwrap().mouse());
                        let interval =
                            Duration::from_millis(if poll { FALLBACK_POLL_MS } else { AUDIT_MS });
                        let mut reinstall = resumed;
                        if resumed {
                            LISTENER.with(|slot| slot.borrow_mut().as_mut().unwrap().baseline());
                        } else if audit_at.elapsed() >= interval {
                            LISTENER.with(|slot| {
                                let mut slot = slot.borrow_mut();
                                let listener = slot.as_mut().unwrap();
                                if listener.config.is_none()
                                    || (!poll
                                        && listener.last_edge.elapsed() < Duration::from_millis(20))
                                {
                                    return;
                                }
                                let snapshot = listener.snapshot();
                                if snapshot.pressed == listener.chord.pressed {
                                    listener.mismatch = None;
                                } else if poll || listener.mismatch == Some(snapshot.pressed) {
                                    let before = listener.chord.down();
                                    listener.chord = snapshot;
                                    listener.mismatch = None;
                                    if listener.chord.down() != before {
                                        listener.emit(listener.chord.down(), false, Source::Poll);
                                    }
                                    // A silently removed hook must not remain the only input source.
                                    reinstall = !poll;
                                } else {
                                    listener.mismatch = Some(snapshot.pressed);
                                }
                            });
                            audit_at = Instant::now();
                        }
                        if reinstall {
                            hook = Hook::install().ok();
                        }
                        let timeout = interval
                            .saturating_sub(audit_at.elapsed())
                            .as_millis()
                            .max(1) as u32;
                        if unsafe {
                            MsgWaitForMultipleObjectsEx(
                                None,
                                timeout,
                                QS_ALLINPUT,
                                MWMO_INPUTAVAILABLE,
                            )
                        } == WAIT_FAILED
                        {
                            break;
                        }
                    }
                    drop(hook);
                    LISTENER.with(|slot| *slot.borrow_mut() = None);
                })
                .context("Cannot start shortcut event thread")?;
            let thread_id = match ready.recv_timeout(Duration::from_secs(1)) {
                Ok(id) => id,
                Err(error) => {
                    stop.store(true, Ordering::Release);
                    return Err(error).context("Shortcut event thread did not start");
                }
            };
            Ok(Self {
                commands,
                events,
                epoch: 0,
                config: None,
                thread_id,
                alive,
                stop,
                overflow,
                armed: false,
            })
        }

        pub fn alive(&self) -> bool {
            self.alive.load(Ordering::Acquire)
        }

        pub fn accept(&mut self, event: &KeyEvent) -> bool {
            if event.epoch != self.epoch {
                return false;
            }
            if event.baseline {
                self.armed = true;
            }
            true
        }

        pub fn armed(&self) -> bool {
            self.armed && self.alive()
        }

        pub fn overflowed(&self) -> bool {
            self.overflow.swap(false, Ordering::AcqRel)
        }

        pub fn configure(&mut self, config: Option<&Hotkey>, reset: bool) -> Result<()> {
            if !reset && self.config.as_ref() == config {
                return Ok(());
            }
            if reset {
                // Make room for the new baseline before asking the producer to resynchronize.
                discard_pending(&self.events);
            }
            self.epoch += 1;
            self.armed = false;
            self.config = config.cloned();
            if self
                .commands
                .try_send(Configuration {
                    epoch: self.epoch,
                    hotkey: self.config.clone(),
                })
                .is_err()
            {
                bail!("Shortcut configuration queue unavailable");
            }
            unsafe {
                PostThreadMessageW(self.thread_id, WAKE_MESSAGE, WPARAM(0), LPARAM(0))?;
            }
            Ok(())
        }
    }

    impl Drop for Monitor {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WAKE_MESSAGE, WPARAM(0), LPARAM(0));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::hotkey::{DirectHotkey, Mode};

        fn listener(capacity: usize) -> (Listener, Receiver<KeyEvent>) {
            let (output, receiver) = mpsc::sync_channel(capacity);
            let (wake, _) = Wake::channel();
            let config = Hotkey {
                keys: vec![0xa4],
                taps: 2,
                mode: Mode::Hold,
            };
            (
                Listener {
                    chord: Chord::snapshot(&config, |_| false),
                    config: Some(config),
                    epoch: 1,
                    last_edge: Instant::now(),
                    mismatch: None,
                    output,
                    overflow: Arc::new(AtomicBool::new(false)),
                    wake,
                },
                receiver,
            )
        }

        #[test]
        fn hook_edge_queue_preserves_rapid_taps_without_synthesizing_system_input() {
            let (mut listener, events) = listener(64);
            let mut shortcut = DirectHotkey::default();
            shortcut.configure(
                listener.config.clone(),
                false,
                None,
                runtime::now() + 2500,
                runtime::clock_us(),
            );
            shortcut.baseline(false, runtime::now());
            listener.key(0xa4, true);
            listener.key(0xa4, false);
            listener.key(0xa4, true);
            let frames: Vec<_> = events.try_iter().collect();
            assert_eq!(frames.len(), 3);
            for event in frames {
                assert_eq!(event.trigger.source, Source::Hook);
                shortcut.sample(event.down, event.at_ms, event.trigger);
            }
            assert!(shortcut.intent().1.is_some());
        }

        #[test]
        fn a_full_queue_reports_loss_instead_of_blocking_the_keyboard_callback() {
            let (mut listener, events) = listener(2);
            for _ in 0..10_000 {
                listener.key(0xa4, true);
                listener.key(0xa4, false);
            }
            assert!(listener.overflow.load(Ordering::Acquire));
            assert_eq!(events.try_iter().count(), 2);
        }

        #[test]
        fn resynchronization_drains_a_full_queue_so_its_baseline_can_be_delivered() {
            let (mut listener, events) = listener(2);
            for _ in 0..3 {
                listener.key(0xa4, true);
                listener.key(0xa4, false);
            }
            assert!(listener.overflow.swap(false, Ordering::AcqRel));
            discard_pending(&events);
            listener.epoch += 1;
            listener.emit(false, true, Source::Poll);
            let baseline = events.try_recv().unwrap();
            assert!(baseline.baseline);
            assert_eq!(baseline.epoch, 2);
            assert!(!listener.overflow.load(Ordering::Acquire));
        }

        #[test]
        fn mouse_chords_do_not_mix_hook_edges_with_the_poll_fallback() {
            let (mut listener, events) = listener(64);
            let config = Hotkey {
                keys: vec![0xa4, 0x04],
                taps: 1,
                mode: Mode::Hold,
            };
            listener.chord = Chord::snapshot(&config, |_| false);
            listener.config = Some(config);
            listener.key(0xa4, true);
            listener.key(0x04, true);
            assert!(events.try_recv().is_err());
            assert!(listener.mouse());
        }

        #[test]
        fn an_unconfigured_native_listener_starts_and_stops_without_changing_input_or_audio() {
            let (wake, _) = Wake::channel();
            let mut monitor = Monitor::start(wake).unwrap();
            let alive = Arc::clone(&monitor.alive);
            monitor.configure(None, true).unwrap();
            let event = monitor.events.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(event.baseline && !event.down);
            assert!(monitor.accept(&event));
            let old = KeyEvent { epoch: 0, ..event };
            assert!(!monitor.accept(&old));
            drop(monitor);
            let deadline = Instant::now() + Duration::from_secs(2);
            while alive.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!alive.load(Ordering::Acquire));
        }
    }
}

#[cfg(windows)]
pub use native::Monitor;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::Mode;

    fn config(keys: Vec<u16>) -> Hotkey {
        Hotkey {
            keys,
            taps: 2,
            mode: Mode::Hold,
        }
    }

    #[test]
    fn physical_left_alt_is_not_right_alt_or_an_auto_repeat() {
        let mut chord = Chord::snapshot(&config(vec![0xa4]), |_| false);
        assert_eq!(chord.event(0xa5, true), None);
        assert_eq!(chord.event(0xa4, true), Some(true));
        assert_eq!(chord.event(0xa4, true), None);
        assert_eq!(chord.event(0xa4, false), Some(false));
    }

    #[test]
    fn side_aware_generic_modifiers_keep_the_chord_down_until_both_release() {
        let mut chord = Chord::snapshot(&config(vec![0x11, 0x41]), |_| false);
        assert_eq!(chord.event(0xa2, true), None);
        assert_eq!(chord.event(0x41, true), Some(true));
        assert_eq!(chord.event(0xa3, true), None);
        assert_eq!(chord.event(0xa2, false), None);
        assert_eq!(chord.event(0xa3, false), Some(false));
    }

    #[test]
    fn unrelated_typing_does_not_change_or_accumulate_state() {
        let mut chord = Chord::snapshot(&config(vec![0xa4]), |_| false);
        let baseline = chord.clone();
        for _ in 0..1000 {
            for key in 0x20..0x80 {
                assert_eq!(chord.event(key, true), None);
                assert_eq!(chord.event(key, false), None);
            }
        }
        assert!(chord == baseline);
    }

    #[test]
    fn modifier_scancodes_are_normalized_without_reading_async_state_in_the_hook() {
        assert_eq!(normalize(0x12, 0x38, false), 0xa4);
        assert_eq!(normalize(0x12, 0x38, true), 0xa5);
        assert_eq!(normalize(0x10, 0x36, false), 0xa1);
        assert_eq!(normalize(0x11, 0x1d, true), 0xa3);
    }
}
