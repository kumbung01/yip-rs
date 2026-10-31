//! Effective GRTC/HF ratio measured while the executor can sleep.

pub(crate) const CALIBRATION_US: u32 = 31_250;
pub(crate) const EXPECTED_TICKS: u32 = CALIBRATION_US * 16;

#[derive(Clone, Copy)]
pub(crate) struct GrtcRatio {
    ticks: u32,
}

impl GrtcRatio {
    pub(crate) const fn nominal() -> Self {
        Self {
            ticks: EXPECTED_TICKS,
        }
    }

    pub(crate) fn measured(ticks: u32) -> Option<Self> {
        (450_000..=550_000)
            .contains(&ticks)
            .then_some(Self { ticks })
    }

    pub(crate) fn ticks(self) -> u32 {
        self.ticks
    }

    pub(crate) fn hf_to_grtc(self, us: u32) -> Option<u32> {
        scale(us, EXPECTED_TICKS, self.ticks())
    }

    pub(crate) fn grtc_to_hf(self, us: u32) -> Option<u32> {
        scale(us, self.ticks(), EXPECTED_TICKS)
    }
}

// Exact floor(value * numerator / denominator), without a 64-bit division.
// Both factors stay between 450,000 and 550,000. Splitting at bit 10 keeps
// every intermediate product below u32::MAX, including the remainder terms.
fn scale(value: u32, numerator: u32, denominator: u32) -> Option<u32> {
    let whole = value / denominator;
    let remainder = value % denominator;
    let high = remainder * (numerator >> 10);
    let low = (high % denominator) * 1024 + remainder * (numerator & 1023);
    // Together these terms are less than numerator (at most 550,000).
    let fraction = (high / denominator) * 1024 + low / denominator;
    whole.checked_mul(numerator)?.checked_add(fraction)
}
