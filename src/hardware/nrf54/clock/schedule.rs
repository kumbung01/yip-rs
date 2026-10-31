//! Peripheral scheduling arithmetic, independent of hardware registers.

use super::ratio::GrtcRatio;

const MAX_REFERENCE_AGE_US: u64 = 2_000_000;

pub(crate) fn next_sync(now: u64, previous: u64, period: u32, margin: u32) -> (u64, u32) {
    debug_assert!(period > 0, "sync period must be positive");
    debug_assert!(
        now <= u64::MAX - margin as u64,
        "sync margin overflows time"
    );
    let earliest = now + margin as u64;
    let distance = earliest.saturating_sub(previous);
    let steps = distance.div_ceil(period as u64);
    debug_assert!(steps <= u32::MAX as u64, "sync step count exceeds u32");
    let advance = steps * period as u64;
    debug_assert!(
        previous <= u64::MAX - advance,
        "sync schedule overflows time"
    );
    (previous + advance, steps as u32)
}

pub(crate) fn next_slot(
    now: u64,
    heartbeat: u64,
    loop_us: u32,
    drift_us: i32,
    heartbeat_loops: u32,
    margin_us: u32,
    rx_window_us: u32,
    ratio: GrtcRatio,
) -> Option<(u64, u32)> {
    let elapsed = now.checked_sub(heartbeat)?;
    if elapsed >= MAX_REFERENCE_AGE_US || loop_us == 0 || heartbeat_loops == 0 {
        return None;
    }
    // Only the absolute GRTC timestamps need 64 bits. The age limit above
    // bounds elapsed time; all relative calculations below use microseconds.
    let elapsed = elapsed as u32;
    let earliest = elapsed.checked_add(margin_us)?;
    let earliest_hf = ratio.grtc_to_hf(earliest)?;
    let rx_window_us = i32::try_from(rx_window_us).ok()?;
    let heartbeat_loops = i32::try_from(heartbeat_loops).ok()?;
    let mut loops = earliest_hf.div_ceil(loop_us).max(1);
    for _ in 0..4 {
        let base = i32::try_from(loops.checked_mul(loop_us)?).ok()?;
        let drift = drift_us
            .checked_mul(i32::try_from(loops).ok()?)?
            .div_euclid(heartbeat_loops);
        let advance = base.checked_add(drift)?.checked_sub(rx_window_us)?;
        // Round towards the earlier receive-window edge, including negative drift.
        let offset = ratio.hf_to_grtc(u32::try_from(advance).ok()?)?;
        if offset >= earliest {
            return Some((heartbeat.checked_add(offset as u64)?, loops));
        }
        loops = loops.checked_add(1)?;
    }
    None
}

pub(crate) fn next_prepare(
    now: u64,
    heartbeat: u64,
    loop_us: u32,
    heartbeat_us: u32,
    margin_us: u32,
    ratio: GrtcRatio,
) -> u64 {
    debug_assert!(loop_us > 0, "connection loop must be positive");
    debug_assert!(heartbeat_us > 0, "heartbeat period must be positive");
    debug_assert!(now >= heartbeat, "heartbeat must not be in the future");
    let elapsed = u32::try_from(now - heartbeat).expect("heartbeat reference too old");
    let elapsed_hf = ratio
        .grtc_to_hf(elapsed)
        .expect("heartbeat interval overflow");
    let margin = ratio
        .grtc_to_hf(margin_us)
        .expect("heartbeat margin overflow");
    let earliest = elapsed_hf
        .checked_add(loop_us)
        .and_then(|t| t.checked_add(margin))
        .expect("heartbeat interval overflow");
    let periods = (earliest / heartbeat_us)
        .checked_add(1)
        .expect("heartbeat interval overflow");
    let offset = periods
        .checked_mul(heartbeat_us)
        .and_then(|t| t.checked_sub(loop_us))
        .expect("heartbeat interval overflow");
    heartbeat
        .checked_add(
            ratio
                .hf_to_grtc(offset)
                .expect("heartbeat interval overflow") as u64,
        )
        .expect("heartbeat schedule overflows time")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_is_future_and_preserves_scan_phase() {
        assert_eq!(next_sync(1_000_000, 750_000, 750_000, 100), (1_500_000, 1));
        assert_eq!(next_sync(749_900, 750_000, 750_000, 100), (750_000, 0));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "sync period must be positive")]
    fn sync_rejects_zero_period() {
        next_sync(0, 0, 0, 100);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "sync margin overflows time")]
    fn sync_rejects_time_overflow() {
        next_sync(u64::MAX, 0, 750_000, 100);
    }

    #[test]
    fn receive_window_opens_before_the_remote_slot() {
        assert_eq!(
            next_slot(
                1_000_100,
                1_000_000,
                1000,
                0,
                500,
                100,
                10,
                GrtcRatio::nominal()
            ),
            Some((1_000_990, 1))
        );
        // The 990us edge is too close; skip to the following loop.
        assert_eq!(
            next_slot(
                1_000_900,
                1_000_000,
                1000,
                0,
                500,
                100,
                10,
                GrtcRatio::nominal()
            ),
            Some((1_001_990, 2))
        );
    }

    #[test]
    fn drift_is_applied_per_heartbeat_in_both_directions() {
        assert_eq!(
            next_slot(
                1_498_100,
                1_000_000,
                1000,
                20,
                500,
                100,
                10,
                GrtcRatio::nominal()
            ),
            Some((1_499_009, 499))
        );
        assert_eq!(
            next_slot(
                1_498_100,
                1_000_000,
                1000,
                -20,
                500,
                100,
                10,
                GrtcRatio::nominal()
            ),
            Some((1_498_970, 499))
        );
    }

    #[test]
    fn invalid_or_expired_sync_requires_resynchronization() {
        assert_eq!(
            next_slot(
                3_000_000,
                1_000_000,
                1000,
                0,
                500,
                100,
                10,
                GrtcRatio::nominal()
            ),
            None
        );
        assert_eq!(
            next_slot(0, 1, 1000, 0, 500, 100, 10, GrtcRatio::nominal()),
            None
        );
        assert_eq!(
            next_slot(0, 0, 0, 0, 500, 100, 10, GrtcRatio::nominal()),
            None
        );
        assert_eq!(
            next_slot(0, 0, 1000, 0, 0, 100, 10, GrtcRatio::nominal()),
            None
        );
    }

    #[test]
    fn absolute_grtc_time_does_not_truncate_at_32_bits() {
        let heartbeat = (1u64 << 32) + 1_000_000;
        assert_eq!(
            next_slot(
                heartbeat + 100,
                heartbeat,
                1000,
                0,
                500,
                100,
                10,
                GrtcRatio::nominal()
            ),
            Some((heartbeat + 990, 1))
        );
    }

    #[test]
    fn relative_arithmetic_overflow_requires_resynchronization() {
        assert_eq!(
            next_slot(1, 0, 1000, 0, 500, u32::MAX, 10, GrtcRatio::nominal()),
            None
        );
        assert_eq!(
            next_slot(
                0,
                0,
                i32::MAX as u32 + 1,
                0,
                500,
                100,
                10,
                GrtcRatio::nominal()
            ),
            None
        );
        assert_eq!(
            next_slot(1100, 0, 1000, i32::MAX, 500, 100, 10, GrtcRatio::nominal()),
            None
        );
    }

    #[test]
    fn schedule_near_reference_age_limit_still_fits_32_bits() {
        assert_eq!(
            next_slot(1_999_800, 0, 1000, 0, 500, 100, 10, GrtcRatio::nominal()),
            Some((1_999_990, 2000))
        );
    }

    #[test]
    fn prepare_wakes_one_connection_loop_before_a_future_heartbeat() {
        assert_eq!(
            next_prepare(
                1_100_000,
                1_000_000,
                3500,
                500_000,
                100,
                GrtcRatio::nominal()
            ),
            1_496_500
        );
    }
}
