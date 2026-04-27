use std::time::Instant;

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
