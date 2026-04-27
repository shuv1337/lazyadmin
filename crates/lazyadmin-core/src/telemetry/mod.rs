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
