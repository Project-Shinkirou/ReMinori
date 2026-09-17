use std::time::Duration;
pub struct Scheduler {
    ticks_per_second: u64,
    acc: u128,
    pending: u64,
}
impl Scheduler {
    pub const DEFAULT_TPS: u64 = 100;
    pub fn new(ticks_per_second: u64) -> Self {
        assert!(ticks_per_second > 0, "ticks_per_second 须为正");
        Scheduler {
            ticks_per_second,
            acc: 0,
            pending: 0,
        }
    }
    pub fn ticks_per_second(&self) -> u64 {
        self.ticks_per_second
    }
    pub fn pending(&self) -> u64 {
        self.pending
    }
    pub fn advance(&mut self, delta: Duration) -> u64 {
        self.acc += delta.as_nanos() * self.ticks_per_second as u128;
        let produced = (self.acc / 1_000_000_000) as u64;
        self.acc %= 1_000_000_000;
        self.pending += produced;
        produced
    }
    pub fn consume_one(&mut self) -> bool {
        if self.pending > 0 {
            self.pending -= 1;
            true
        } else {
            false
        }
    }
}

