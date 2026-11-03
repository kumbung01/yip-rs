use crate::config::UpdateResult;
use nonmax::NonMaxI32;

/// Required hardware API. Every chip backend must implement all methods.
pub(crate) trait ClockDriver {
    type Timestamp;
    type Timing;

    fn get_start_ts(&self) -> Self::Timestamp;
    fn start_central(&mut self);
    fn stop_central(&mut self);
    fn set_next_slot_central(&mut self) -> u32;
    fn set_timeout_central(&mut self, bytes: usize) -> u32;

    /// Request HFCLK without blocking.
    fn start_hfclk(&mut self);
    fn calibrate_hfclk(&mut self);
    fn stop_hfclk(&mut self);
    fn start_peripheral(&mut self);
    fn stop_peripheral(&mut self);
    fn set_next_sync(&mut self, immediate: bool) -> (u32, u32);
    fn set_next_pre_slot(&mut self, tm: &Self::Timing);
    fn set_next_slot_peripheral(&mut self, tm: &Self::Timing) -> Result<(u32, u32), ()>;
    fn read_sync(&self) -> u32;
    fn update_sync(&self, tm: &mut Self::Timing) -> UpdateResult;
    fn new_timestamp_by_sync(&self, ts: Self::Timestamp, sync: u32) -> Result<Self::Timestamp, ()>;
}

/// Measure a cold start with the role's idle 1 MHz TIMER, before installing radio routes.
pub(crate) fn measure_hfclk_us(
    timer: &mut embassy_nrf::timer::Timer<'_>,
    stopped: impl Fn() -> bool,
    start: impl FnOnce(),
    ready: impl Fn() -> bool,
) -> u32 {
    const TIMEOUT_US: u32 = 10_000;
    const SETTLE_US: u32 = 5_000;
    timer.clear();
    timer.start();
    let now = || timer.cc(3).capture();
    while !stopped() {
        assert!(now() < TIMEOUT_US, "HFCLK did not stop for calibration");
    }
    // Allow the crystal oscillation to decay after switching away from it.
    let stopped_at = now();
    while now() - stopped_at < SETTLE_US {}
    let started_at = now();
    start();
    while !ready() {
        assert!(now() - started_at < TIMEOUT_US, "HFCLK startup timed out");
    }
    let elapsed = now() - started_at;
    timer.stop();
    timer.clear();

    // A boot sample is not a worst-case measurement. Reserve another 25%.
    elapsed + elapsed.div_ceil(4)
}

#[derive(Default)]
pub(crate) struct Timing<T> {
    pub hb: T,
    // Filter units belong to the backend (16 MHz HF ticks on nRF52, us on nRF54).
    pub drift: Option<NonMaxI32>,
    pub hb_loops: u32,
    pub loop_us: u32,
    pub slot_now: u32,

    pub is_hb: bool,
    pub check_next_slot: bool,
}

impl<T> Timing<T> {
    // Callers bound each residual. The accumulated correction is expected to fit
    // in i32 during normal operation; the addition below has no overflow guard.
    // An exact i32::MAX clears the correction, so the next sample is taken in full.
    pub fn filtered_drift(&self, residual: i32) -> Option<NonMaxI32> {
        let value = self
            .drift
            .map_or(residual, |prev| prev.get() + residual / 4);
        NonMaxI32::new(value)
    }

    pub fn drift(&self) -> i32 {
        self.drift.map_or(0, |value| value.get())
    }

    pub fn drift_reset(&mut self) {
        self.drift = None;
    }
}
