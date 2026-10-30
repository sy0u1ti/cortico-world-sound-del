use crate::runtime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Hook,
    Poll,
    Host,
    Simulation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Trigger {
    pub source: Source,
    pub at_us: u64,
}

impl Trigger {
    pub fn now(source: Source) -> Self {
        Self {
            source,
            at_us: runtime::clock_us(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Latency {
    pub source: Source,
    pub dispatch_ms: f64,
    pub first_mute_ms: Option<f64>,
    pub all_muted_ms: Option<f64>,
}

impl Latency {
    pub fn measured(
        trigger: Trigger,
        received_at: u64,
        first_muted_at: Option<u64>,
        all_muted_at: Option<u64>,
    ) -> Option<Self> {
        let elapsed = |at: u64| at.saturating_sub(trigger.at_us) as f64 / 1000.0;
        let value = Self {
            source: trigger.source,
            dispatch_ms: elapsed(received_at),
            first_mute_ms: first_muted_at.map(elapsed),
            all_muted_ms: all_muted_at.map(elapsed),
        };
        (trigger.at_us <= received_at && value.valid()).then_some(value)
    }

    pub fn valid(&self) -> bool {
        let bounded = |ms: f64| ms.is_finite() && (0.0..60_000.0).contains(&ms);
        bounded(self.dispatch_ms)
            && self
                .first_mute_ms
                .is_none_or(|ms| bounded(ms) && ms >= self.dispatch_ms)
            && self.all_muted_ms.is_none_or(|ms| {
                bounded(ms) && ms >= self.first_mute_ms.unwrap_or(self.dispatch_ms)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_time_is_not_reported_as_completed_muting() {
        let trigger = Trigger {
            source: Source::Hook,
            at_us: 1_000,
        };
        let timing = Latency::measured(trigger, 2_000, Some(6_000), Some(9_000)).unwrap();
        assert_eq!(timing.dispatch_ms, 1.0);
        assert_eq!(timing.first_mute_ms, Some(5.0));
        assert_eq!(timing.all_muted_ms, Some(8.0));
        let failed = Latency::measured(trigger, 2_000, None, None).unwrap();
        assert!(failed.all_muted_ms.is_none());
        assert!(Latency::measured(trigger, 500, None, None).is_none());
    }
}
