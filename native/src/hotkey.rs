use crate::latency::{Source, Trigger};
use serde::{Deserialize, Serialize};

pub const FALLBACK_POLL_MS: u64 = 10;
pub const START_CONFIRM_MS: u64 = 1_200;
pub const RELEASE_GRACE_MS: u64 = 350;
const TAP_MAX_MS: u64 = 300;
const TAP_GAP_MS: u64 = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Hold,
    Toggle,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hotkey {
    pub keys: Vec<u16>,
    pub taps: u8,
    pub mode: Mode,
}

impl Hotkey {
    pub fn valid(&self) -> bool {
        (1..=3).contains(&self.taps)
            && !self.keys.is_empty()
            && self.keys.len() <= 8
            && self.keys.iter().all(|key| {
                matches!(*key,
                0x04..=0x06 | 0x09 | 0x0d | 0x10..=0x14 | 0x20..=0x28
                | 0x2d..=0x2e | 0x30..=0x39 | 0x41..=0x5c | 0x70..=0x87
                | 0x91 | 0xa0..=0xa5 | 0xc0)
            })
    }

    pub fn down(&self) -> bool {
        #[cfg(windows)]
        {
            self.keys.iter().all(|key| unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(i32::from(*key)) < 0
            })
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

#[derive(Default)]
struct Gesture {
    previous: Option<bool>,
    pressed_at: Option<u64>,
    released_at: u64,
    taps: u8,
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Press,
    Release,
}

impl Gesture {
    fn sample(&mut self, down: bool, now: u64, taps: u8) -> Option<Edge> {
        let previous = self.previous.replace(down)?;
        if previous == down {
            return None;
        }
        if down {
            if now.saturating_sub(self.released_at) > TAP_GAP_MS {
                self.taps = 0;
            }
            if self.taps + 1 == taps {
                self.taps = 0;
                self.active = true;
                return Some(Edge::Press);
            }
            self.pressed_at = Some(now);
        } else if self.active {
            self.active = false;
            return Some(Edge::Release);
        } else if let Some(at) = self.pressed_at.take() {
            if now.saturating_sub(at) <= TAP_MAX_MS {
                self.taps += 1;
                self.released_at = now;
            } else {
                self.taps = 0;
            }
        }
        None
    }
}

#[derive(Clone, Copy, Default)]
enum Phase {
    #[default]
    Idle,
    Pending {
        until: u64,
    },
    Confirmed,
    Closing {
        until: u64,
        confirmed: bool,
    },
}

#[derive(Default)]
pub struct DirectHotkey {
    config: Option<Hotkey>,
    gesture: Gesture,
    phase: Phase,
    remote_active: bool,
    suppressed: bool,
    expired_wait: bool,
    lease_until: u64,
    generation: u64,
    id: Option<String>,
    trigger: Option<Trigger>,
    cancelled_at_us: u64,
}

impl DirectHotkey {
    pub fn configure(
        &mut self,
        config: Option<Hotkey>,
        active: bool,
        remote_id: Option<String>,
        expires_at: u64,
        received_at_us: u64,
    ) {
        if self.config != config {
            self.config = config;
            self.gesture = Gesture::default();
            self.end();
            self.suppressed = false;
        }
        self.lease_until = expires_at;
        if self.remote_active && !active {
            self.cancelled_at_us = received_at_us;
        }
        self.remote_active = active;
        if active && self.expired_wait {
            self.suppressed = true;
            self.end();
            return;
        }
        if !active {
            self.expired_wait = false;
            self.suppressed = false;
            if matches!(
                self.phase,
                Phase::Confirmed
                    | Phase::Closing {
                        confirmed: true,
                        ..
                    }
            ) {
                self.end();
            }
        } else if !self.suppressed {
            match self.phase {
                Phase::Idle => {
                    self.phase = Phase::Confirmed;
                    self.id = remote_id;
                    self.trigger = Some(Trigger {
                        source: Source::Host,
                        at_us: received_at_us,
                    });
                }
                Phase::Pending { .. } => self.phase = Phase::Confirmed,
                Phase::Closing { until, .. } => {
                    self.phase = Phase::Closing {
                        until,
                        confirmed: true,
                    }
                }
                Phase::Confirmed => {}
            }
        }
    }

    pub fn configured(&self) -> bool {
        self.config.is_some()
    }

    pub fn config(&self) -> Option<&Hotkey> {
        self.config.as_ref()
    }

    pub fn trigger(&self) -> Option<Trigger> {
        self.trigger
    }

    pub fn fresh(&self, now: u64) -> bool {
        now < self.lease_until
    }

    pub fn tick(&mut self, now: u64) {
        if now >= self.lease_until {
            self.expired_wait |= self.id.is_some();
            self.config = None;
            self.gesture = Gesture::default();
            self.lease_until = 0;
            self.remote_active = false;
            self.suppressed = self.expired_wait;
            self.end();
            return;
        }
        match self.phase {
            Phase::Pending { until }
                if now >= until
                    && !matches!(self.config.as_ref().map(|key| key.mode), Some(Mode::Hold)) =>
            {
                self.suppressed = self.remote_active;
                self.end();
            }
            Phase::Closing { until, .. } if now >= until => {
                self.suppressed = self.remote_active;
                self.end();
            }
            _ => {}
        }
    }

    pub fn baseline(&mut self, down: bool, _now: u64) {
        self.gesture = Gesture {
            previous: Some(down),
            active: down && self.id.is_some(),
            ..Gesture::default()
        };
        if matches!(
            self.phase,
            Phase::Pending { .. }
                | Phase::Closing {
                    confirmed: false,
                    ..
                }
        ) {
            self.end();
        }
    }

    pub fn sample(&mut self, down: bool, now: u64, trigger: Trigger) {
        if let Some(config) = &self.config {
            let mode = config.mode;
            if let Some(edge) = self.gesture.sample(down, now, config.taps) {
                match (edge, mode, self.phase) {
                    (Edge::Press, _, Phase::Idle)
                        if !self.expired_wait && trigger.at_us > self.cancelled_at_us =>
                    {
                        self.suppressed = false;
                        self.generation += 1;
                        self.id = Some(format!("hotkey-{}", self.generation));
                        self.trigger = Some(trigger);
                        self.phase = Phase::Pending {
                            until: now.saturating_add(START_CONFIRM_MS),
                        };
                    }
                    (Edge::Press, Mode::Hold, Phase::Closing { .. }) => {
                        self.phase = if self.remote_active {
                            Phase::Confirmed
                        } else {
                            Phase::Pending {
                                until: now.saturating_add(START_CONFIRM_MS),
                            }
                        };
                    }
                    (Edge::Release, Mode::Hold, Phase::Pending { .. }) => {
                        self.phase = Phase::Closing {
                            until: now.saturating_add(RELEASE_GRACE_MS),
                            confirmed: false,
                        };
                    }
                    (Edge::Release, Mode::Hold, Phase::Confirmed) => {
                        self.phase = Phase::Closing {
                            until: now.saturating_add(RELEASE_GRACE_MS),
                            confirmed: true,
                        };
                    }
                    // Toggle release is not a stop; the host's next closed gate ends recording.
                    _ => {}
                }
            }
        }
    }

    #[cfg(test)]
    pub fn poll(&mut self, now: u64, simulated_down: Option<bool>) {
        self.tick(now);
        self.sample(
            simulated_down.unwrap(),
            now,
            Trigger {
                source: Source::Simulation,
                at_us: now * 1000,
            },
        );
    }

    pub fn intent(&self) -> (u64, Option<String>) {
        (
            if self.id.is_some() {
                self.lease_until
            } else {
                0
            },
            self.id.clone(),
        )
    }

    fn end(&mut self) {
        self.phase = Phase::Idle;
        self.id = None;
        self.trigger = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotkey(mode: Mode, taps: u8) -> Hotkey {
        Hotkey {
            keys: vec![0xa4],
            taps,
            mode,
        }
    }
    fn setup(mode: Mode, taps: u8) -> DirectHotkey {
        let mut state = DirectHotkey::default();
        state.configure(Some(hotkey(mode, taps)), false, None, 5000, 0);
        state.poll(0, Some(false));
        state
    }

    #[test]
    fn press_mutes_before_any_open_notification() {
        let mut state = setup(Mode::Hold, 1);
        state.poll(10, Some(true));
        assert_eq!(state.intent(), (5000, Some("hotkey-1".into())));
        state.configure(Some(hotkey(Mode::Hold, 1)), false, None, 6000, 0);
        assert!(
            state.intent().1.is_some(),
            "An earlier closed snapshot must not cancel a pending key"
        );
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            6000,
            0,
        );
        assert_eq!(state.intent().1.as_deref(), Some("hotkey-1"));
    }

    #[test]
    fn double_and_triple_taps_start_only_on_the_final_press() {
        for taps in [2, 3] {
            let mut state = setup(Mode::Hold, taps);
            for tap in 0..taps - 1 {
                state.poll(10 + u64::from(tap) * 200, Some(true));
                assert!(state.intent().1.is_none());
                state.poll(100 + u64::from(tap) * 200, Some(false));
            }
            state.poll(10 + u64::from(taps - 1) * 200, Some(true));
            assert!(state.intent().1.is_some());
        }
    }

    #[test]
    fn queued_edges_at_the_same_timestamp_preserve_the_whole_double_tap() {
        let mut state = setup(Mode::Hold, 2);
        state.poll(10, Some(true));
        state.poll(10, Some(false));
        state.poll(10, Some(true));
        assert!(state.intent().1.is_some());
    }

    #[test]
    fn resynchronization_does_not_complete_a_partial_gesture_or_treat_a_held_key_as_new() {
        let mut state = setup(Mode::Hold, 2);
        state.poll(10, Some(true));
        state.poll(20, Some(false));
        state.baseline(true, 30);
        state.poll(40, Some(false));
        state.poll(50, Some(true));
        assert!(state.intent().1.is_none());
        state.poll(60, Some(false));
        state.poll(70, Some(true));
        assert!(state.intent().1.is_some());
    }

    #[test]
    fn queued_press_from_before_host_cancellation_cannot_restart_muting() {
        let mut state = setup(Mode::Hold, 1);
        state.poll(10, Some(true));
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            5000,
            0,
        );
        let old = Trigger {
            source: Source::Hook,
            at_us: 20_000,
        };
        state.configure(Some(hotkey(Mode::Hold, 1)), false, None, 5000, 30_000);
        state.baseline(false, 30);
        state.sample(true, 20, old);
        assert!(state.intent().1.is_none());
        state.poll(50, Some(false));
        state.poll(60, Some(true));
        assert!(state.intent().1.is_some());
    }

    #[test]
    fn already_held_key_is_ignored_on_start_and_configuration_change() {
        let mut state = DirectHotkey::default();
        state.configure(Some(hotkey(Mode::Hold, 1)), false, None, 5000, 0);
        state.poll(10, Some(true));
        state.poll(20, Some(false));
        assert!(state.intent().1.is_none());
        state.poll(30, Some(true));
        assert!(state.intent().1.is_some());
        state.configure(Some(hotkey(Mode::Hold, 2)), false, None, 5000, 0);
        state.poll(40, Some(true));
        state.poll(50, Some(false));
        state.poll(60, Some(true));
        assert!(state.intent().1.is_none());
    }

    #[test]
    fn long_or_widely_spaced_taps_do_not_trigger() {
        let mut state = setup(Mode::Hold, 2);
        state.poll(10, Some(true));
        state.poll(400, Some(false));
        state.poll(450, Some(true));
        assert!(state.intent().1.is_none());
        state.poll(500, Some(false));
        state.poll(1000, Some(true));
        assert!(state.intent().1.is_none());
    }

    #[test]
    fn release_waits_for_close_or_a_bounded_tail_grace() {
        let mut state = setup(Mode::Hold, 1);
        state.poll(10, Some(true));
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            5000,
            0,
        );
        state.poll(50, Some(false));
        assert!(state.intent().1.is_some());
        state.configure(Some(hotkey(Mode::Hold, 1)), false, None, 5000, 0);
        assert!(state.intent().1.is_none());
        state.poll(60, Some(true));
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            5000,
            0,
        );
        state.poll(100, Some(false));
        state.poll(100 + RELEASE_GRACE_MS, Some(false));
        assert!(state.intent().1.is_none());
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            6000,
            0,
        );
        assert!(
            state.intent().1.is_none(),
            "A stuck old host state cannot rearm after release timeout"
        );
    }

    #[test]
    fn an_unconfirmed_toggle_gesture_expires() {
        let mut state = setup(Mode::Toggle, 1);
        state.poll(10, Some(true));
        state.poll(10 + START_CONFIRM_MS, Some(true));
        state.poll(1500, Some(true));
        assert!(state.intent().1.is_none());
    }

    #[test]
    fn held_key_is_authoritative_until_release_even_if_the_start_notification_is_slow() {
        let mut state = setup(Mode::Hold, 2);
        state.poll(10, Some(true));
        state.poll(100, Some(false));
        state.poll(180, Some(true));
        state.configure(Some(hotkey(Mode::Hold, 2)), false, None, 7000, 0);
        state.poll(2000, Some(true));
        assert!(state.intent().1.is_some());
        state.poll(2100, Some(false));
        state.poll(2100 + RELEASE_GRACE_MS, Some(false));
        assert!(state.intent().1.is_none());
    }

    #[test]
    fn a_fresh_press_is_not_suppressed_by_the_previous_release_timeout() {
        let mut state = setup(Mode::Hold, 1);
        state.poll(10, Some(true));
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            5000,
            0,
        );
        state.poll(100, Some(false));
        state.poll(500, Some(false));
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            6000,
            0,
        );
        state.poll(550, Some(true));
        assert_eq!(state.intent().1.as_deref(), Some("hotkey-2"));
    }

    #[test]
    fn host_cancel_while_held_does_not_remute() {
        let mut state = setup(Mode::Hold, 1);
        state.poll(10, Some(true));
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            5000,
            0,
        );
        state.configure(Some(hotkey(Mode::Hold, 1)), false, None, 5000, 0);
        state.poll(50, Some(true));
        assert!(state.intent().1.is_none());
    }

    #[test]
    fn toggle_release_does_not_end_a_confirmed_recording() {
        let mut state = setup(Mode::Toggle, 1);
        state.poll(10, Some(true));
        state.configure(
            Some(hotkey(Mode::Toggle, 1)),
            true,
            Some("recording".into()),
            5000,
            0,
        );
        state.poll(50, Some(false));
        state.poll(500, Some(true));
        assert!(state.intent().1.is_some());
        state.configure(Some(hotkey(Mode::Toggle, 1)), false, None, 5000, 0);
        assert!(state.intent().1.is_none());
    }

    #[test]
    fn lost_host_lease_disarms_even_while_key_is_held() {
        let mut state = setup(Mode::Hold, 1);
        state.poll(10, Some(true));
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            5000,
            0,
        );
        state.poll(5000, Some(true));
        assert!(state.intent().1.is_none());
        assert!(!state.configured());
        state.configure(
            Some(hotkey(Mode::Hold, 1)),
            true,
            Some("recording".into()),
            8000,
            0,
        );
        state.poll(5010, Some(true));
        assert!(
            state.intent().1.is_none(),
            "An active-only reconnect cannot rearm an expired session"
        );
        state.configure(Some(hotkey(Mode::Hold, 1)), false, None, 8000, 0);
        state.poll(5020, Some(false));
        state.poll(5030, Some(true));
        assert!(state.intent().1.is_some());
    }

    #[test]
    fn host_notifications_still_work_when_no_supported_hotkey_is_available() {
        let mut state = DirectHotkey::default();
        state.configure(None, true, Some("recording".into()), 5000, 0);
        state.poll(10, Some(false));
        assert_eq!(state.intent().1.as_deref(), Some("recording"));
        state.configure(None, false, None, 5000, 0);
        assert!(state.intent().1.is_none());
    }

    #[test]
    fn invalid_key_sets_are_rejected() {
        assert!(
            !Hotkey {
                keys: vec![],
                taps: 1,
                mode: Mode::Hold
            }
            .valid()
        );
        assert!(
            !Hotkey {
                keys: vec![0xffff],
                taps: 1,
                mode: Mode::Hold
            }
            .valid()
        );
        assert!(!hotkey(Mode::Hold, 4).valid());
        assert!(hotkey(Mode::Hold, 2).valid());
    }
}
