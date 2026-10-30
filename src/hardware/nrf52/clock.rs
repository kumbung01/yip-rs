use crate::hardware::api::{ClockDriver, ClockTimestamp};

use embassy_nrf::{
    Peri,
    rtc::{CompareChannel as RtcCc, Instance as RtcInstance, Interrupt, Rtc},
    timer::{Instance as TimerInstance, Timer},
};

use crate::config::*;
use crate::pdu::*;

#[derive(Debug, PartialEq, PartialOrd, Clone, Copy, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct Timestamp {
    rtc: u32,
    timer: u32,
}

impl ClockTimestamp for Timestamp {
    #[cfg(feature = "defmt")]
    fn unscaled(&self) -> Timestamp {
        Timestamp {
            rtc: self.rtc,
            timer: unscale_timer(self.timer),
        }
    }
}

pub(crate) struct Nrf52Clock {
    pub(crate) rtc: Rtc<'static>,
    pub(crate) timer: Timer<'static>,
    ts: Timestamp,
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

        debug!("timer prescale: {}", timer.regs().prescaler().read().0);

        Self {
            rtc,
            timer,
            ts: Timestamp::default(),
        }
    }
}

// central impl

// peripheral impl

pub(crate) type Timing = crate::hardware::api::clock::Timing<Timestamp>;

fn unscale_timer(cc: u32) -> u32 {
    (cc + TIMER_TICK_SCALED / 2) / TIMER_TICK_SCALED
}

impl ClockDriver for Nrf52Clock {
    type Timestamp = Timestamp;
    type Timing = Timing;
    #[cfg(feature = "defmt")]
    type PeripheralInstant = u32;

    fn get_start_ts(&self) -> Timestamp {
        self.ts
    }

    #[cfg(feature = "defmt")]
    fn get_start_hw(&mut self) -> Timestamp {
        Timestamp {
            rtc: self.rtc.regs().cc(0).read().0,
            timer: self.timer.cc(0).read(),
        }
    }

    fn start_central(&mut self) {
        self.timer.start();
    }

    fn stop_central(&mut self) {
        self.timer.stop();
        self.timer.clear();
    }

    fn set_next_slot_central(&mut self) -> u32 {
        let now_us = self.timer.cc(3).capture();
        let ts = self.ts.timer;
        let elapsed_us = (now_us - ts) as u32;
        let radio_margin_us = TIMESLOT_MARGIN_US;
        let slot_diff = (elapsed_us + radio_margin_us).div_ceil(YIP_SLOTSIZE_US as u32);
        let tx_start = ts.wrapping_add((slot_diff).wrapping_mul(YIP_SLOTSIZE_US as u32));

        self.ts.timer = tx_start;
        self.timer.cc(0).write(tx_start);

        slot_diff
    }

    fn set_timeout_central(&mut self, bytes: usize) -> u32 {
        const HEADER_BITS: u32 = PduByte0::LENGTH_BIT_COUNT as u32 + PduByte1::S1_BITS as u32;

        const HEADER_TICKS: u32 = {
            assert!(RADIO_BITS_PER_US > 0);

            (HEADER_BITS * TIMER_TICKS_PER_US).div_ceil(RADIO_BITS_PER_US)
        };

        let tx_ticks = ADDR_DELAY_TICKS
            + HEADER_TICKS
            + (bytes as u32 + RADIO_CRC_LEN_BYTES) * TIMER_TICKS_PER_BYTE;
        let rx_ticks = ADDR_DELAY_TICKS + PACKETBUF_AIRTIME_TICKS;

        let timeout_ticks = tx_ticks + rx_ticks + TIMEOUT_MARGIN_TIMER_TICKS;

        assert!(
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
        debug!("start peripheral clock");

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

    #[cfg(feature = "defmt")]
    fn get_now_peripheral(&self) -> u32 {
        self.rtc.read()
    }

    fn set_next_sync(&mut self) -> (u32, u32) {
        let sync_duration_scaled =
            us_to_timer_ticks(PERIPHEARL_SYNC_DURATION_US) as u32 * TIMER_TICK_SCALED;

        let rtc_now = self.rtc.read();
        let elapsed = rtc_elapsed(rtc_now, self.ts.rtc);

        let distance_scaled =
            (elapsed) * RTC_TICK_SCALED as u32 + RTC_MARGIN_SCALED - self.ts.timer;
        let slot_diff = distance_scaled.div_ceil(sync_duration_scaled);

        let advance_scaled = slot_diff * sync_duration_scaled + self.ts.timer;

        let rtc_delta = advance_scaled / RTC_TICK_SCALED as u32;
        let remainder = advance_scaled % RTC_TICK_SCALED as u32;

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
        // let prepare_ticks = us_to_rtc_ticks(1_000) as u32;
        let prepare_ticks = tm.loop_ticks_scaled / RTC_TICK_SCALED as u32;
        let hb_rtc_tick = us_to_rtc_ticks(HEARTBEAT_DURATION_US) as u32;

        let rtc_now = self.rtc.read();
        let elapsed = rtc_elapsed(rtc_now, tm.hb.rtc);
        let hb_diff = (elapsed + prepare_ticks + RADIO_MARGIN_RTC_TICKS) / hb_rtc_tick + 1;
        let wake_rtc = rtc_wrap(
            tm.hb
                .rtc
                .wrapping_add(hb_diff * hb_rtc_tick)
                .wrapping_sub(prepare_ticks),
        );

        self.rtc
            .set_compare(RtcCc::_0, wake_rtc)
            .expect("rtc value must within 24bit");

        // debug!("now {} start {}", rtc_now, wake_rtc);
    }

    fn set_next_slot_peripheral(&mut self, tm: &Timing) -> Result<(u32, u32), ()> {
        let rtc_now = self.rtc.read();
        let elapsed_rtc = rtc_elapsed(rtc_now, tm.hb.rtc);

        // Keep arithmetic within 32 bits; an expired reference or an
        // unrepresentable intermediate requires synchronization again.
        let next_slot = (|| -> Option<(u32, u32, u32)> {
            if elapsed_rtc >= 65_536 {
                return None;
            }

            let rtc_elapsed_scaled = elapsed_rtc * RTC_TICK_SCALED as u32;
            let distance_scaled = rtc_elapsed_scaled + RTC_MARGIN_SCALED - tm.hb.timer;
            let loop_ticks_scaled = tm.loop_ticks_scaled;
            let hb_loops = i32::try_from(tm.hb_loops).ok()?;
            let min_rtc_delta = (elapsed_rtc + RADIO_MARGIN_RTC_TICKS) as i32;
            let mut loop_diff = distance_scaled.div_ceil(loop_ticks_scaled).max(1);

            for _ in 0..4 {
                let drift_now_scaled = tm
                    .get_drift_scaled()
                    .checked_mul(i32::try_from(loop_diff).ok()?)?
                    / hb_loops;
                let base_scaled = loop_diff
                    .checked_mul(loop_ticks_scaled)?
                    .checked_add(tm.hb.timer)?;
                let advance_scaled = i32::try_from(base_scaled)
                    .ok()?
                    .checked_add(drift_now_scaled)?
                    .checked_sub(
                        (us_to_timer_ticks(RX_WINDOW_MARGIN_US) as u32 * TIMER_TICK_SCALED) as i32,
                    )?;
                let rtc_delta = advance_scaled.div_euclid(RTC_TICK_SCALED as i32);

                if rtc_delta >= min_rtc_delta {
                    let remainder = advance_scaled.rem_euclid(RTC_TICK_SCALED as i32) as u32;
                    return Some((rtc_delta as u32, remainder, loop_diff));
                }

                loop_diff = loop_diff.checked_add(1)?;
            }

            None
        })();

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

        // debug!(
        //     "now {} start rtc {} timer {} timeout {}",
        //     rtc_now, rtc_next, timer_cc, timeout
        // );

        Ok((loop_diff, timeout))
    }

    fn read_sync(&self) -> u32 {
        self.timer.cc(2).read()
    }

    fn update_sync(&self, tm: &mut Timing) -> UpdateResult {
        let sync = self.read_sync() as i32 - ADDR_DELAY_TICKS as i32;
        let drift = sync - unscale_timer(self.ts.timer) as i32 - RX_WINDOW_MARGIN_US as i32;
        debug!("drift: {} us", drift);

        if !tm.is_hb {
            return UpdateResult::Skipped;
        }

        if drift.abs() > DRIFT_LIMIT_US as i32 {
            return UpdateResult::Failed;
        }

        let Ok(new_ts) = Self::update_timestamp_by_sync(self.ts, sync as u32) else {
            return UpdateResult::Failed;
        };

        tm.set_drift(drift);
        tm.hb = new_ts;

        UpdateResult::Updated
    }

    fn update_timestamp_by_sync(ts: Timestamp, sync: u32) -> Result<Timestamp, ()> {
        let sync_scaled = sync.checked_mul(TIMER_TICK_SCALED).ok_or(())?;

        let rtc_delta = sync_scaled.div_euclid(RTC_TICK_SCALED as u32);
        let remainder = sync_scaled.rem_euclid(RTC_TICK_SCALED as u32);

        Ok(Timestamp {
            rtc: rtc_wrap(ts.rtc.wrapping_add(rtc_delta)),
            timer: remainder,
        })
    }
}
