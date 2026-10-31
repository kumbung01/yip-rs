/// HF ticks measured over 1,024 RTC ticks at a 16 MHz TIMER frequency.
/// Remainders use 1,024 units per HF tick (16,384 units per microsecond).
#[derive(Clone, Copy)]
pub(crate) struct RtcRatio {
    measured: u32,
}

pub(crate) const HF_TICKS_PER_US: u32 = 16;
pub(crate) const REMAINDER_PER_HF_TICK: u32 = 1024;

impl RtcRatio {
    pub(crate) const fn nominal() -> Self {
        Self { measured: 500_000 }
    }

    pub(crate) fn measured(ticks: u32) -> Option<Self> {
        // Reject a missing/misconfigured clock and bound all split products below u32::MAX.
        (450_000..=550_000)
            .contains(&ticks)
            .then_some(Self { measured: ticks })
    }

    pub(crate) fn ticks(self) -> u32 {
        self.measured
    }

    pub(crate) fn split_hf(self, ticks: u32) -> (u32, u32) {
        let measured = self.ticks();
        let whole = ticks / measured;
        let fraction = (ticks % measured) * REMAINDER_PER_HF_TICK;
        // Even u32::MAX HF ticks produce fewer than 10 million RTC ticks.
        let rtc = whole * REMAINDER_PER_HF_TICK + fraction / measured;
        (rtc, fraction % measured)
    }

    #[cfg(test)]
    pub(crate) fn split_us(self, us: u32) -> Option<(u32, u32)> {
        Some(self.split_hf(us.checked_mul(HF_TICKS_PER_US)?))
    }

    /// Add an HF interval to a normalized RTC remainder.
    pub(crate) fn advance(self, remainder: u32, ticks: u32) -> (u32, u32) {
        debug_assert!(remainder < self.measured);
        let (rtc, fraction) = self.split_hf(ticks);
        let fraction = fraction + remainder;
        (rtc + fraction / self.measured, fraction % self.measured)
    }

    /// Apply a signed residual phase correction, expressed in 16 MHz ticks.
    pub(crate) fn adjust(self, time: (u32, u32), ticks: i32) -> Option<(u32, u32)> {
        debug_assert!(time.1 < self.measured);
        let (rtc, fraction) = self.split_hf(ticks.unsigned_abs());
        if ticks >= 0 {
            let remainder = time.1 + fraction;
            Some((
                time.0.checked_add(rtc + remainder / self.measured)?,
                remainder % self.measured,
            ))
        } else if time.1 >= fraction {
            Some((time.0.checked_sub(rtc)?, time.1 - fraction))
        } else {
            Some((
                time.0.checked_sub(rtc + 1)?,
                self.measured + time.1 - fraction,
            ))
        }
    }

    /// Convert a bounded RTC interval to HF ticks, rounding down without a wide product.
    /// Callers use fresh scan/heartbeat references (about two seconds at most).
    pub(crate) fn rtc_to_hf(self, rtc: u32) -> u32 {
        debug_assert!(rtc <= (u32::MAX / 550_000) * REMAINDER_PER_HF_TICK);
        let whole = (rtc / REMAINDER_PER_HF_TICK) * self.measured;
        let fraction = (rtc % REMAINDER_PER_HF_TICK) * self.measured;
        whole + fraction / REMAINDER_PER_HF_TICK
    }

    /// Choose a connected RX window relative to the last accepted heartbeat.
    pub(crate) fn next_slot(
        self,
        remainder: u32,
        elapsed_rtc: u32,
        loop_us: u32,
        hb_loops: u32,
        drift_hf: i32,
        rtc_margin: u32,
        rx_margin_us: u32,
    ) -> Option<(u32, u32, u32)> {
        if elapsed_rtc >= 65_536 || loop_us == 0 || hb_loops == 0 {
            return None;
        }
        debug_assert!(rtc_margin <= 65_536);
        let min_rtc_delta = elapsed_rtc + rtc_margin;
        let earliest_hf = self.rtc_to_hf(min_rtc_delta);
        let distance = earliest_hf.saturating_sub(remainder / REMAINDER_PER_HF_TICK);
        // Sync validates the loop against the heartbeat period before storing it.
        // The caller supplies a constant RX margin; these conversions fit u32/i32.
        debug_assert!(loop_us <= u32::MAX / HF_TICKS_PER_US);
        debug_assert!(hb_loops <= i32::MAX as u32);
        debug_assert!(rx_margin_us <= i32::MAX as u32 / HF_TICKS_PER_US);
        let loop_hf = loop_us * HF_TICKS_PER_US;
        let hb_loops = hb_loops as i32;
        let rx_margin = (rx_margin_us * HF_TICKS_PER_US) as i32;
        let mut loop_diff = distance.div_ceil(loop_hf).max(1);
        for _ in 0..4 {
            // loop_hf >= 16 bounds loop_diff below i32::MAX, including retries.
            let drift = drift_hf.checked_mul(loop_diff as i32)? / hb_loops;
            let base = self.advance(remainder, loop_diff.checked_mul(loop_hf)?);
            if let Some((rtc, fraction)) = self.adjust(base, drift.checked_sub(rx_margin)?) {
                if rtc >= min_rtc_delta {
                    return Some((rtc, fraction, loop_diff));
                }
            }
            // loop_hf >= 16, so distance.div_ceil(loop_hf) + four retries fits u32.
            loop_diff += 1;
        }
        None
    }
}
