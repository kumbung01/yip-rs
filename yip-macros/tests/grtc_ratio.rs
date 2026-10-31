#[path = "../../src/hardware/nrf54/clock/ratio.rs"]
mod ratio;
#[path = "../../src/hardware/nrf54/clock/schedule.rs"]
mod schedule;

use ratio::GrtcRatio;

#[test]
fn conversions_match_wide_reference() {
    assert!(GrtcRatio::measured(0).is_none());
    assert!(GrtcRatio::measured(449_999).is_none());
    assert!(GrtcRatio::measured(550_001).is_none());
    for ticks in [450_000, 499_939, 500_000, 500_250, 550_000] {
        let ratio = GrtcRatio::measured(ticks).unwrap();
        assert_eq!(ratio.ticks(), ticks);
        for us in (0..=2_000_000).step_by(137) {
            assert_eq!(
                ratio.hf_to_grtc(us),
                Some((us as u64 * 500_000 / ticks as u64) as u32)
            );
            assert_eq!(
                ratio.grtc_to_hf(us),
                Some((us as u64 * ticks as u64 / 500_000) as u32)
            );
        }
        for us in [u32::MAX, u32::MAX - 500_000] {
            let hf = us as u64 * 500_000 / ticks as u64;
            let grtc = us as u64 * ticks as u64 / 500_000;
            assert_eq!(ratio.hf_to_grtc(us), u32::try_from(hf).ok());
            assert_eq!(ratio.grtc_to_hf(us), u32::try_from(grtc).ok());
        }
    }
}

#[test]
fn calibrated_windows_preserve_hf_period_and_grtc_headroom() {
    let hb = (1u64 << 32) + 1_000_000;
    for ticks in [499_750, 499_939, 500_000, 500_250] {
        let ratio = GrtcRatio::measured(ticks).unwrap();
        for elapsed in [0, 900, 498_100, 999_800, 1_999_000] {
            for drift in [-20, 0, 20] {
                let (at, loops) =
                    schedule::next_slot(hb + elapsed, hb, 1000, drift, 500, 100, 20, ratio)
                        .unwrap();
                assert!(at >= hb + elapsed + 100);
                let offset_hf = loops as i32 * 1000 + (drift * loops as i32).div_euclid(500) - 20;
                assert_eq!(at, hb + ratio.hf_to_grtc(offset_hf as u32).unwrap() as u64);
            }
        }
        let at = schedule::next_prepare(hb + 100_000, hb, 1000, 500_000, 100, ratio);
        assert_eq!(at, hb + ratio.hf_to_grtc(499_000).unwrap() as u64);
    }
}
