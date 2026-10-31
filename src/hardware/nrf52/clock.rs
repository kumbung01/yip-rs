use crate::hardware::api::ClockDriver;

use embassy_nrf::{
    Peri,
    rtc::{CompareChannel as RtcCc, Instance as RtcInstance, Interrupt, Rtc},
    timer::{Instance as TimerInstance, Timer},
};

use super::rtc_ratio::{HF_TICKS_PER_US, REMAINDER_PER_HF_TICK, RtcRatio};
use crate::config::*;
use crate::pdu::*;

#[derive(Debug, PartialEq, PartialOrd, Clone, Copy, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct Timestamp {
    rtc: u32,
    timer: u32,
}

pub(crate) struct Nrf52Clock {
    pub(crate) rtc: Rtc<'static>,
    pub(crate) timer: Timer<'static>,
    ts: Timestamp,
    pub(crate) ratio: RtcRatio,
}

impl Nrf52Clock {
    pub(crate) fn new<R, T>(rtc: Peri<'static, R>, timer: Peri<'static, T>) -> Self
    where
        R: RtcInstance,
        T: TimerInstance,
    {
        let rtc = Rtc::new(rtc, RTC_PRESCALE).expect("RTC init failed");
        let mut timer = Timer::new(timer);
        timer.set_frequency(TIMER_FREQUENCY);
        timer.regs().bitmode().write(|w| {
            w.set_bitmode(TIMER_BITMODE);
        });

        Self {
            rtc,
            timer,
            ts: Timestamp::default(),
            ratio: RtcRatio::nominal(),
        }
    }
}

// central impl

// peripheral impl

pub(crate) type Timing = crate::hardware::api::clock::Timing<Timestamp>;

impl Timing {
    fn set_drift(&mut self, drift: i32) {
        // Residual central/peripheral rate error, after RTC/HF calibration, in 16 MHz ticks.
        let error_scaled = drift * HF_TICKS_PER_US as i32;
        if let Some(value) = &mut self.drift {
            *value += error_scaled / 4;
        } else {
            self.drift = Some(error_scaled);
        }
    }

    fn get_drift_scaled(&self) -> i32 {
        self.drift.unwrap_or(0)
    }
}

fn unscale_timer(cc: u32) -> u32 {
    (cc + TIMER_TICK_SCALED / 2) / TIMER_TICK_SCALED
}

impl ClockDriver for Nrf52Clock {
    type Timestamp = Timestamp;
    type Timing = Timing;

    fn get_start_ts(&self) -> Timestamp {
        self.ts
    }

    fn start_central(&mut self) {
        self.timer.start();
    }

    fn stop_central(&mut self) {
        self.timer.stop();
        self.timer.clear();
        self.ts.timer = 0;
    }

    fn set_next_slot_central(&mut self) -> u32 {
        let now_us = self.timer.cc(3).capture();
        let ts = self.ts.timer;
        let radio_margin_us = TIMESLOT_MARGIN_US;
        let elapsed_us = now_us.wrapping_sub(ts);
        let slot_diff = (elapsed_us + radio_margin_us).div_ceil(YIP_SLOTSIZE_US as u32);
        let tx_start = ts.wrapping_add((slot_diff).wrapping_mul(YIP_SLOTSIZE_US as u32));

        self.ts.timer = tx_start;
        // Diagnostic event flags must belong to this reservation, not the previous TX.
        #[cfg(any(feature = "defmt", feature = "log"))]
        self.timer.regs().events_compare(0).write_value(0);
        self.timer.cc(0).write(tx_start);

        slot_diff
    }

    fn set_timeout_central(&mut self, bytes: usize) -> u32 {
        const HEADER_BITS: u32 = PduByte0::LENGTH_BIT_COUNT as u32 + PduByte1::S1_BITS as u32;

        const HEADER_TICKS: u32 = {
            debug_assert!(RADIO_BITS_PER_US > 0);

            (HEADER_BITS * TIMER_TICKS_PER_US).div_ceil(RADIO_BITS_PER_US)
        };

        let tx_ticks = ADDR_DELAY_TICKS
            + HEADER_TICKS
            + (bytes as u32 + RADIO_CRC_LEN_BYTES) * TIMER_TICKS_PER_BYTE;
        let rx_ticks = ADDR_DELAY_TICKS + PACKETBUF_AIRTIME_TICKS;

        let timeout_ticks = tx_ticks + rx_ticks + TIMEOUT_MARGIN_TIMER_TICKS;

        debug_assert!(
            timeout_ticks
                < us_to_timer_ticks(YIP_SLOTSIZE_US as u32) as u32 - TIMEOUT_MARGIN_TIMER_TICKS,
            "invalid timeout ticks"
        );

        self.timer
            .cc(1)
            .write(self.ts.timer.wrapping_add(timeout_ticks));

        timeout_ticks
    }

    fn start_peripheral(&mut self) {
        self.timer.cc(1).short_compare_clear();
        self.timer.cc(1).short_compare_stop();

        self.rtc.enable_event(Interrupt::Compare0);
        self.rtc.enable();
    }

    fn stop_peripheral(&mut self) {
        self.rtc.disable();
        self.rtc.disable_event(Interrupt::Compare0);

        self.timer.stop();
        self.timer.clear();
    }

    fn set_next_sync(&mut self, immediate: bool) -> (u32, u32) {
        // Every RTC-triggered window needs a fresh relative counter.
        self.timer.stop();
        self.timer.clear();

        let rtc_now = self.rtc.read();
        if immediate {
            // Arm the first scan after hardware setup headroom, not a scan period.
            self.ts = Timestamp {
                rtc: rtc_wrap(rtc_now.wrapping_add(RADIO_MARGIN_RTC_TICKS)),
                timer: 0,
            };
            self.rtc
                .set_compare(RtcCc::_0, self.ts.rtc)
                .expect("rtc value must within 24bit");
            self.timer.cc(0).write(1);
            let timeout =
                1 + us_to_timer_ticks(PERIPHEARL_SYNC_DURATION_US - PERIPHERAL_SYNC_TIMETOUT_US)
                    as u32;
            self.timer.cc(1).write(timeout);
            return (0, timeout);
        }
        let mut elapsed = rtc_elapsed(rtc_now, self.ts.rtc);
        if elapsed >= 65_536 {
            // start/stop can leave the old reference ahead of the stopped RTC.
            self.ts = Timestamp {
                rtc: rtc_now,
                timer: 0,
            };
            elapsed = 0;
        }
        let period_hf = PERIPHEARL_SYNC_DURATION_US * HF_TICKS_PER_US;
        let earliest_hf = self.ratio.rtc_to_hf(elapsed + RADIO_MARGIN_RTC_TICKS);
        let distance = earliest_hf.saturating_sub(self.ts.timer / REMAINDER_PER_HF_TICK);
        let mut slot_diff = distance.div_ceil(period_hf).max(1);
        let (rtc_delta, remainder) = loop {
            let next = self.ratio.advance(self.ts.timer, slot_diff * period_hf);
            if next.0 >= elapsed + RADIO_MARGIN_RTC_TICKS {
                break next;
            }
            slot_diff += 1;
        };

        self.ts = Timestamp {
            rtc: rtc_wrap(self.ts.rtc.wrapping_add(rtc_delta)),
            timer: remainder,
        };

        self.rtc
            .set_compare(RtcCc::_0, self.ts.rtc)
            .expect("rtc value must within 24bit");

        let timer_cc = unscale_timer(remainder).max(1);
        self.timer.cc(0).write(timer_cc);

        let timeout = timer_cc + us_to_timer_ticks(PERIPHEARL_SYNC_DURATION_US) as u32
            - us_to_timer_ticks(PERIPHERAL_SYNC_TIMETOUT_US) as u32;
        self.timer.cc(1).write(timeout);

        (slot_diff, timeout)
    }

    fn set_next_pre_slot(&mut self, tm: &Timing) {
        let rtc_now = self.rtc.read();
        let elapsed = rtc_elapsed(rtc_now, tm.hb.rtc);
        let loop_hf = tm.loop_us * HF_TICKS_PER_US;
        let hb_hf = tm.hb_loops * loop_hf;
        let earliest_hf = self
            .ratio
            .rtc_to_hf(elapsed + RADIO_MARGIN_RTC_TICKS)
            .saturating_sub(tm.hb.timer / REMAINDER_PER_HF_TICK);
        let hb_diff = (earliest_hf + loop_hf) / hb_hf + 1;
        let (wake_delta, _) = self.ratio.advance(tm.hb.timer, hb_diff * hb_hf - loop_hf);
        let wake_rtc = rtc_wrap(tm.hb.rtc.wrapping_add(wake_delta));

        self.rtc
            .set_compare(RtcCc::_0, wake_rtc)
            .expect("rtc value must within 24bit");
    }

    fn set_next_slot_peripheral(&mut self, tm: &Timing) -> Result<(u32, u32), ()> {
        // Reset before writing the next window's relative CC values.
        self.timer.stop();
        self.timer.clear();

        let rtc_now = self.rtc.read();
        let elapsed_rtc = rtc_elapsed(rtc_now, tm.hb.rtc);

        // Keep arithmetic within 32 bits; an expired reference or an
        // unrepresentable intermediate requires synchronization again.
        let next_slot = self.ratio.next_slot(
            tm.hb.timer,
            elapsed_rtc,
            tm.loop_us,
            tm.hb_loops,
            tm.get_drift_scaled(),
            RADIO_MARGIN_RTC_TICKS,
            RX_WINDOW_MARGIN_US,
        );

        let Some((rtc_delta, remainder, loop_diff)) = next_slot else {
            return Err(());
        };

        let rtc_next = rtc_wrap(tm.hb.rtc.wrapping_add(rtc_delta));
        self.rtc
            .set_compare(RtcCc::_0, rtc_next)
            .expect("rtc val must within 24 bit");

        let timer_cc = unscale_timer(remainder).max(1);
        self.timer.cc(0).write(timer_cc);

        let timeout = ADDR_DELAY_TICKS
            + PACKETBUF_AIRTIME_TICKS
            + TIMEOUT_MARGIN_TIMER_TICKS
            + us_to_timer_ticks(RX_WINDOW_MARGIN_US) as u32;
        self.timer.cc(1).write(timer_cc + timeout);

        self.ts = Timestamp {
            rtc: rtc_next,
            timer: remainder,
        };

        Ok((loop_diff, timeout))
    }

    fn read_sync(&self) -> u32 {
        self.timer.cc(2).read()
    }

    fn update_sync(&self, tm: &mut Timing) -> UpdateResult {
        let sync = self.read_sync() as i32 - ADDR_DELAY_TICKS as i32;
        let drift = sync - unscale_timer(self.ts.timer) as i32 - RX_WINDOW_MARGIN_US as i32;

        if !tm.is_hb {
            return UpdateResult::Skipped;
        }

        if drift.abs() > DRIFT_LIMIT_US as i32 {
            return UpdateResult::Failed;
        }

        let Ok(new_ts) = self.update_timestamp_by_sync(self.ts, sync as u32) else {
            return UpdateResult::Failed;
        };

        tm.set_drift(drift);
        tm.hb = new_ts;

        UpdateResult::Updated
    }

    fn update_timestamp_by_sync(&self, ts: Timestamp, sync: u32) -> Result<Timestamp, ()> {
        // Capture is relative to the RTC edge, not to ts.timer: do not add the old remainder.
        let hf_ticks = sync
            .checked_mul(HF_TICKS_PER_US / TIMER_TICKS_PER_US)
            .ok_or(())?;
        let (rtc_delta, remainder) = self.ratio.split_hf(hf_ticks);

        Ok(Timestamp {
            rtc: rtc_wrap(ts.rtc.wrapping_add(rtc_delta)),
            timer: remainder,
        })
    }
}
