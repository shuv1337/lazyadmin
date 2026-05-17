use std::time::Instant;

pub const ADAPTER_WATCH_START: &str = "adapter.watch.start";
pub const ADAPTER_WATCH_EVENT: &str = "adapter.watch.event";
pub const ADAPTER_WATCH_STOP: &str = "adapter.watch.stop";
pub const ADAPTER_SOCKDIAG_DISCOVER: &str = "adapter.sockdiag.discover";
pub const LISTENER_DUALSTACK_PROBE: &str = "listener.dualstack.probe";

#[derive(Debug)]
pub struct Timer {
    start: Instant,
}
impl Timer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }
    pub fn duration_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn span_event_names_match_documented_constants() {
        // These constant names are part of the operational contract — operators
        // grep for them in journalctl. Pin them here so a rename is caught.
        assert_eq!(ADAPTER_WATCH_START, "adapter.watch.start");
        assert_eq!(ADAPTER_WATCH_EVENT, "adapter.watch.event");
        assert_eq!(ADAPTER_WATCH_STOP, "adapter.watch.stop");
        assert_eq!(ADAPTER_SOCKDIAG_DISCOVER, "adapter.sockdiag.discover");
        assert_eq!(LISTENER_DUALSTACK_PROBE, "listener.dualstack.probe");
    }

    #[test]
    fn telemetry_event_names_are_dot_separated_lowercase() {
        for name in [
            ADAPTER_WATCH_START,
            ADAPTER_WATCH_EVENT,
            ADAPTER_WATCH_STOP,
            ADAPTER_SOCKDIAG_DISCOVER,
            LISTENER_DUALSTACK_PROBE,
        ] {
            assert!(
                name.contains('.'),
                "telemetry constant should be dotted: {name}"
            );
            assert_eq!(
                name.to_ascii_lowercase(),
                name,
                "telemetry constant should be lowercase: {name}"
            );
            assert!(!name.contains(' '), "no spaces in event name: {name}");
        }
    }

    #[test]
    fn timer_returns_zero_or_more_milliseconds_immediately() {
        let t = Timer::start();
        let d = t.duration_ms();
        // Elapsed can be 0 for a very fast call but never negative.
        assert!(d < 5_000, "freshly started timer should be tiny, got {d}ms");
    }

    #[test]
    fn timer_observes_elapsed_time() {
        let t = Timer::start();
        sleep(Duration::from_millis(20));
        let d = t.duration_ms();
        assert!(d >= 15, "timer should observe ~20ms, got {d}ms");
    }

    #[test]
    fn timer_is_monotonic_across_two_reads() {
        let t = Timer::start();
        let d1 = t.duration_ms();
        sleep(Duration::from_millis(5));
        let d2 = t.duration_ms();
        assert!(
            d2 >= d1,
            "second reading must not go backwards (d1={d1} d2={d2})"
        );
    }

    #[test]
    fn timer_is_debug_renderable() {
        let t = Timer::start();
        // Just confirm Debug is implemented and produces a non-empty string.
        let s = format!("{:?}", t);
        assert!(s.contains("Timer"));
    }
}
