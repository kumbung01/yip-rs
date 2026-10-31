// Exercise the MCU-independent 32-bit conversion with a 64-bit arithmetic oracle.
#[path = "../../src/hardware/nrf52/rtc_ratio.rs"]
mod rtc_ratio;

use rtc_ratio::RtcRatio;

#[test]
fn measured_ratios_match_wide_reference_across_long_intervals() {
    for measured in [450_000, 499_750, 499_939, 500_000, 500_250, 550_000] {
        let ratio = RtcRatio::measured(measured).unwrap();
        assert_eq!(ratio.ticks(), measured);
        for us in [
            0, 1, 20, 500, 1000, 31_250, 499_500, 500_000, 750_000, 2_000_000,
        ] {
            let expected = us as u64 * 16_384;
            assert_eq!(
                ratio.split_us(us),
                Some((
                    (expected / measured as u64) as u32,
                    (expected % measured as u64) as u32
                ))
            );
        }
        for rtc in [0, 1, 1023, 1024, 16_384, 65_536] {
            assert_eq!(
                ratio.rtc_to_hf(rtc),
                (rtc as u64 * measured as u64 / 1024) as u32
            );
        }
    }
}

#[test]
fn fractional_addition_and_signed_phase_correction_preserve_time() {
    for measured in [499_939, 500_000, 500_250] {
        let ratio = RtcRatio::measured(measured).unwrap();
        for fraction in [0, 1, measured - 1] {
            for ticks in [0, 1, 320, 8_000_000, 12_000_000] {
                let expected = fraction as u64 + ticks as u64 * 1024;
                let actual = ratio.advance(fraction, ticks);
                assert_eq!(
                    actual,
                    (
                        (expected / measured as u64) as u32,
                        (expected % measured as u64) as u32
                    )
                );
                for correction in [-4000, -320, -1, 0, 1, 320, 4000] {
                    let expected = expected as i64 + correction as i64 * 1024;
                    let adjusted = ratio.adjust(actual, correction);
                    if expected < 0 {
                        assert!(adjusted.is_none());
                    } else {
                        assert_eq!(
                            adjusted,
                            Some((
                                (expected / measured as i64) as u32,
                                (expected % measured as i64) as u32
                            ))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn calibration_removes_the_observed_initial_and_heartbeat_offsets() {
    let ratio = RtcRatio::measured(499_939).unwrap();
    let nominal = RtcRatio::nominal();
    // A sync packet arrives 174 ms after the RTC edge that started scanning.
    let sync = ratio.split_us(174_000).unwrap();
    let old = nominal.split_us(174_000).unwrap();
    let old_hf = (old.0 as u64 * ratio.ticks() as u64 + old.1 as u64) / 1024;
    let actual_hf = 174_000 * 16;
    assert!((330..=350).contains(&(actual_hf - old_hf as u32)));
    let (delta, remainder) = ratio.advance(sync.1, 500_000 * 16);
    let expected = ratio.split_us(674_000).unwrap();
    assert_eq!((sync.0 + delta, remainder), expected);
    // Calibrated 500 ms corresponds to more RTC ticks than the nominal 16,384.
    assert!(ratio.split_us(500_000).unwrap().0 > 16_384);
}

#[test]
fn invalid_measurements_and_unrepresentable_intervals_are_rejected() {
    assert!(RtcRatio::measured(0).is_none());
    assert!(RtcRatio::measured(449_999).is_none());
    assert!(RtcRatio::measured(550_001).is_none());
    assert!(RtcRatio::nominal().split_us(u32::MAX).is_none());
}

#[test]
fn full_width_hf_intervals_and_normalized_remainders_always_fit() {
    for measured in [450_000, 499_939, 500_000, 550_000] {
        let ratio = RtcRatio::measured(measured).unwrap();
        for ticks in [0, 1, measured - 1, measured, u32::MAX - 1, u32::MAX] {
            let expected = ticks as u64 * 1024;
            assert_eq!(
                ratio.split_hf(ticks),
                (
                    (expected / measured as u64) as u32,
                    (expected % measured as u64) as u32,
                )
            );
            for remainder in [0, measured - 1] {
                let expected = expected + remainder as u64;
                assert_eq!(
                    ratio.advance(remainder, ticks),
                    (
                        (expected / measured as u64) as u32,
                        (expected % measured as u64) as u32,
                    )
                );
            }
        }
    }
}

#[test]
fn next_rx_windows_keep_compare_margin_and_residual_drift_in_correct_units() {
    for measured in [499_750, 499_939, 500_000, 500_250] {
        let ratio = RtcRatio::measured(measured).unwrap();
        for remainder in [0, 1, measured - 1] {
            for elapsed in [0, 30, 32, 16_350, 16_384, 32_768, 65_535] {
                for drift_hf in [-4000, -16, 0, 16, 4000] {
                    let (rtc, fraction, loops) = ratio
                        .next_slot(remainder, elapsed, 1000, 500, drift_hf, 3, 20)
                        .unwrap();
                    assert!(rtc >= elapsed + 3);
                    assert!(fraction < measured);
                    let correction = drift_hf * loops as i32 / 500;
                    let expected = remainder as i64
                        + (loops as i64 * 1000 * 16 + correction as i64 - 20 * 16) * 1024;
                    assert_eq!(rtc as i64 * measured as i64 + fraction as i64, expected);
                    // The timestamp's RTC component can safely wrap at 24 bits.
                    let reference = 0x00ff_fff0u32;
                    let scheduled = reference.wrapping_add(rtc) & 0x00ff_ffff;
                    assert_eq!(scheduled.wrapping_sub(reference) & 0x00ff_ffff, rtc);
                }
            }
        }
    }
    let ratio = RtcRatio::nominal();
    assert!(ratio.next_slot(0, 65_536, 1000, 500, 0, 3, 20).is_none());
    assert!(ratio.next_slot(0, 0, 0, 500, 0, 3, 20).is_none());
    assert!(ratio.next_slot(0, 0, 1000, 0, 0, 3, 20).is_none());
}
