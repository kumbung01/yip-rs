use crate::config::UpdateResult;

/// Required hardware API. Every chip backend must implement all methods.
pub(crate) trait ClockDriver {
    type Timestamp: ClockTimestamp;
    type Timing;
    #[cfg(feature = "defmt")]
    type PeripheralInstant;

    fn get_start_ts(&self) -> Self::Timestamp;
    #[cfg(feature = "defmt")]
    fn get_start_hw(&mut self) -> Self::Timestamp;
    fn start_central(&mut self);
    fn stop_central(&mut self);
    fn set_next_slot_central(&mut self) -> u32;
    fn set_timeout_central(&mut self, bytes: usize) -> u32;

    fn start_peripheral(&mut self);
    fn stop_peripheral(&mut self);
    #[cfg(feature = "defmt")]
    fn get_now_peripheral(&self) -> Self::PeripheralInstant;
    fn set_next_sync(&mut self) -> (u32, u32);
    fn set_next_pre_slot(&mut self, tm: &Self::Timing);
    fn set_next_slot_peripheral(&mut self, tm: &Self::Timing) -> Result<(u32, u32), ()>;
    fn read_sync(&self) -> u32;
    fn update_sync(&self, tm: &mut Self::Timing) -> UpdateResult;
    fn update_timestamp_by_sync(ts: Self::Timestamp, sync: u32) -> Result<Self::Timestamp, ()>;
}

/// Chip-specific timestamp conversion used by role diagnostics.
pub(crate) trait ClockTimestamp: Sized {
    #[cfg(feature = "defmt")]
    fn unscaled(&self) -> Self;
}

#[derive(Default)]
pub(crate) struct Timing<T> {
    pub hb: T,
    // Filter units belong to the backend (RTC fixed point on nRF52, us on nRF54).
    pub drift: Option<i32>,
    pub hb_loops: u32,
    pub loop_us: u32,
    pub slot_now: u32,

    pub is_hb: bool,
    pub check_next_slot: bool,
}

impl<T> Timing<T> {
    pub fn drift_reset(&mut self) {
        self.drift = None;
    }
}
