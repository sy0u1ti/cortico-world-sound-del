use std::sync::atomic::{AtomicU8, Ordering};

const ARMED: u8 = 1;
const OVERRIDDEN: u8 = 2;
const CLOSED: u8 = 4;

/// One acquisition, never reused for a later recording or endpoint registration.
#[derive(Default)]
pub struct MuteObservation(AtomicU8);

impl MuteObservation {
    pub fn notify(&self, own_context: bool, muted: bool) {
        if own_context && muted {
            self.0.fetch_or(ARMED, Ordering::SeqCst);
        } else if !own_context && !muted {
            let state = self.0.load(Ordering::SeqCst);
            if state & ARMED != 0 && state & CLOSED == 0 {
                self.0.fetch_or(OVERRIDDEN, Ordering::SeqCst);
            }
        }
    }

    pub fn overridden(&self) -> bool {
        self.0.load(Ordering::SeqCst) & OVERRIDDEN != 0
    }

    pub fn close(&self) {
        self.0.fetch_or(CLOSED, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_mute_volume_changes_do_not_relinquish_ownership() {
        let observation = MuteObservation::default();
        observation.notify(false, false);
        observation.notify(true, true);
        assert!(!observation.overridden());
    }

    #[test]
    fn external_unmute_then_remute_is_retained() {
        let observation = MuteObservation::default();
        observation.notify(true, true);
        observation.notify(false, false);
        observation.notify(false, true);
        assert!(observation.overridden());
    }

    #[test]
    fn earlier_acquisition_cannot_arm_a_new_observer() {
        let old = MuteObservation::default();
        let current = MuteObservation::default();
        old.notify(true, true);
        old.notify(false, false);
        current.notify(false, true);
        current.notify(false, false);
        assert!(old.overridden());
        assert!(!current.overridden());
    }

    #[test]
    fn late_self_notification_cannot_reopen_a_closed_observer() {
        let observation = MuteObservation::default();
        observation.close();
        observation.notify(true, true);
        observation.notify(false, false);
        assert!(!observation.overridden());
    }

    #[test]
    fn a_notification_flood_has_constant_storage() {
        let observation = MuteObservation::default();
        observation.notify(true, true);
        for _ in 0..100_000 {
            observation.notify(false, false);
            observation.notify(false, true);
        }
        assert!(observation.overridden());
        assert_eq!(std::mem::size_of::<MuteObservation>(), 1);
    }
}
