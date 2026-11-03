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

    fn start_peripheral(&mut self);
    fn stop_peripheral(&mut self);
    fn set_next_sync(&mut self, immediate: bool) -> (u32, u32);
    fn set_next_pre_slot(&mut self, tm: &Self::Timing);
    fn set_next_slot_peripheral(&mut self, tm: &Self::Timing) -> Result<(u32, u32), ()>;
    fn read_sync(&self) -> u32;
    fn update_sync(&self, tm: &mut Self::Timing) -> UpdateResult;
    fn new_timestamp_by_sync(&self, ts: Self::Timestamp, sync: u32) -> Result<Self::Timestamp, ()>;
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
