use crate::hardware::api::ClockDriver;

use core::marker::PhantomData;
use embassy_time::Instant;

use embassy_nrf::{Peri, PeripheralType, peripherals, timer::Timer};

use crate::config::*;
use crate::pdu::*;

#[path = "clock/ratio.rs"]
pub(crate) mod ratio;
#[path = "clock/schedule.rs"]
mod schedule;
use ratio::GrtcRatio;

// About three 32.768 kHz ticks, matching the nRF52 scheduling headroom.
const SCHEDULE_MARGIN_US: u32 = 100;

/*
 * Clock handles all timing related functions.
 */

pub trait GrtcChannel: PeripheralType + 'static {
    const NUMBER: usize;
}

macro_rules! impl_grtc_channels {
    ($($number:literal),* $(,)?) => {
        paste::paste! {
            $(
                impl GrtcChannel for peripherals::[<GRTC_CH $number>] {
                    const NUMBER: usize = $number;
                }
            )*
        }
    };
}

// CC11 belongs to Embassy's time driver; some HAL releases also hide CH1.
// Expose only channels available in both the released and local HAL.
impl_grtc_channels!(0, 2, 3, 4, 5, 6, 7, 8, 9, 10);

struct GrtcCc<'d> {
    number: usize,
    _p: PhantomData<&'d ()>,
}

impl<'d> GrtcCc<'d> {
    pub fn new<G: GrtcChannel>(_token: Peri<'d, G>) -> Self {
        Self {
            number: G::NUMBER,
            _p: PhantomData,
        }
    }

    fn disable(&mut self) {
        let grtc = embassy_nrf::pac::GRTC;
        // Writing CCL alone disarms this channel, without stopping Embassy's GRTC.
        let low = grtc.cc(self.number).ccl().read();
        grtc.cc(self.number).ccl().write_value(low);
        grtc.events_compare(self.number).write_value(0);
    }

    pub fn set_compare(&mut self, timestamp: u64) {
        let cc = embassy_nrf::pac::GRTC.cc(self.number);

        debug_assert!(timestamp < (1u64 << 52), "GRTC timestamp exceeds 52 bits");
        embassy_nrf::pac::GRTC
            .events_compare(self.number)
            .write_value(0);
        // CCL disables comparison; writing CCH re-enables it.
        cc.ccl().write_value(timestamp as u32);
        cc.cch().write(|w| {
            w.set_cch(((timestamp >> 32) & 0xF_FFFF) as u32);
        });
    }
}

// Clock 내부에 저장, 혹은 peripheral 전용 TimingContext 에 저장되는 내용.
#[derive(Debug, PartialEq, PartialOrd, Clone, Copy, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct Timestamp {
    pub(crate) us: u64,
}

pub(crate) struct Nrf54Clock {
    grtc: GrtcCc<'static>,
    pub(crate) timer: Timer<'static>,
    ts: Timestamp,
    ratio: GrtcRatio,
}

impl Nrf54Clock {
    pub fn new<G: GrtcChannel>(
        grtc: Peri<'static, G>,
        timer: Peri<'static, peripherals::TIMER10>,
    ) -> Self {
        let mut timer = Timer::new(timer);

        // Work around the released HAL's 16 MHz assumption for TIMER10.
        // TIMER10 runs from 32 MHz: PRESCALER=5 gives a 1 MHz counter.
        const {
            debug_assert!(
                TIMER_TICK_HZ == 1_000_000,
                "update TIMER10 prescaler for the configured tick rate"
            );
        }
        timer.regs().prescaler().write(|w| w.set_prescaler(5));
        timer.regs().bitmode().write(|w| {
            w.set_bitmode(TIMER_BITMODE);
        });

        Self {
            grtc: GrtcCc::new(grtc),
            timer: timer,
            ts: Timestamp { us: 0 },
            ratio: GrtcRatio::nominal(),
        }
    }
}

// hw specific impl
impl Nrf54Clock {
    pub(crate) fn grtc_num(&self) -> usize {
        self.grtc.number
    }

    pub(crate) fn set_calibration_compare(&mut self, at: u64) {
        self.grtc.set_compare(at);
    }

    pub(crate) fn finish_calibration(&mut self, ticks: u32) -> bool {
        self.grtc.disable();
        if let Some(ratio) = GrtcRatio::measured(ticks) {
            self.ratio = ratio;
            true
        } else {
            false
        }
    }
}

// central impl

pub(crate) type Timing = crate::hardware::api::clock::Timing<Timestamp>;

impl Timing {
    fn set_drift(&mut self, drift_us: i32) {
        if let Some(value) = &mut self.drift {
            *value += drift_us / 4;
        } else {
            self.drift = Some(drift_us);
        }
    }

    fn get_drift_us(&self) -> i32 {
        self.drift.unwrap_or(0)
    }
}

impl ClockDriver for Nrf54Clock {
    type Timestamp = Timestamp;
    type Timing = Timing;

    fn get_start_ts(&self) -> Timestamp {
        self.ts
    }

    fn start_central(&mut self) {
        // self.timer.cc(0).short_compare_stop();
        // self.timer.cc(0).short_compare_clear();
    }

    fn stop_central(&mut self) {
        self.grtc.disable();
        self.timer.stop();
        self.timer.clear();
    }

    fn set_next_slot_central(&mut self) -> u32 {
        let now_us = Instant::now().as_micros();
        let radio_margin_us = ISR_MARGIN_US;
        let elapsed_us = (now_us - self.ts.us) as u32;
        let slot_diff = (elapsed_us + radio_margin_us).div_ceil(YIP_SLOTSIZE_US as u32);
        let tx_start = self
            .ts
            .us
            .wrapping_add((slot_diff).wrapping_mul(YIP_SLOTSIZE_US as u32) as u64);

        self.ts.us = tx_start;

        self.grtc.set_compare(tx_start);

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
        let rx_ticks = ADDR_DELAY_TICKS
            + HEADER_TICKS
            + (YIP_SUB_PACKET_SIZE as u32 + RADIO_CRC_LEN_BYTES) * TIMER_TICKS_PER_BYTE;

        let timeout_ticks = tx_ticks + rx_ticks + TIMEOUT_MARGIN_TIMER_TICKS;
        debug_assert!(
            timeout_ticks < us_to_timer_ticks(YIP_SLOTSIZE_US as u32 - TIMESLOT_MARGIN_US) as u32,
            "timetout_ticks: {}, slotsize timeout: {}",
            timeout_ticks,
            us_to_timer_ticks(YIP_SLOTSIZE_US as u32 - TIMESLOT_MARGIN_US) as u32
        );

        self.timer.cc(0).write(timeout_ticks);

        timeout_ticks
    }

    fn start_peripheral(&mut self) {
        // GRTC is owned and started by Embassy; only our CC and TIMER10 are reset.
        self.grtc.disable();
        self.timer.stop();
        self.timer.clear();
        // CC1 owns peripheral timeout cleanup, independent of DPPI gating.
        // Early CRC completion may leave TIMER running until CC1 or the next
        // receive-window setup, which also stops and clears it explicitly.
        self.timer.regs().shorts().modify(|w| {
            w.set_compare_stop(1, true);
            w.set_compare_clear(1, true);
        });
        self.ts.us = Instant::now().as_micros();
    }

    fn stop_peripheral(&mut self) {
        self.grtc.disable();
        self.timer.stop();
        self.timer.clear();
    }

    fn set_next_sync(&mut self, immediate: bool) -> (u32, u32) {
        let now = Instant::now().as_micros();
        let (at, steps) = if immediate {
            // Keep only the headroom needed to install the RX routes.
            (now + SCHEDULE_MARGIN_US as u64, 0)
        } else {
            schedule::next_sync(
                now,
                self.ts.us,
                self.ratio
                    .hf_to_grtc(PERIPHEARL_SYNC_DURATION_US)
                    .expect("sync interval overflow"),
                SCHEDULE_MARGIN_US,
            )
        };
        self.timer.stop();
        self.timer.clear();
        self.timer.cc(2).write(0);
        let timeout =
            (PERIPHEARL_SYNC_DURATION_US - PERIPHERAL_SYNC_TIMETOUT_US) * TIMER_TICKS_PER_US;
        self.timer.cc(1).write(timeout);
        self.ts.us = at;
        self.grtc.set_compare(at);
        (steps, timeout)
    }

    fn set_next_pre_slot(&mut self, tm: &Timing) {
        let at = schedule::next_prepare(
            Instant::now().as_micros(),
            tm.hb.us,
            tm.loop_us,
            HEARTBEAT_DURATION_US,
            SCHEDULE_MARGIN_US,
            self.ratio,
        );
        self.grtc.set_compare(at);
    }

    fn set_next_slot_peripheral(&mut self, tm: &Timing) -> Result<(u32, u32), ()> {
        let now = Instant::now().as_micros();
        let next_slot = schedule::next_slot(
            now,
            tm.hb.us,
            tm.loop_us,
            tm.get_drift_us(),
            tm.hb_loops,
            SCHEDULE_MARGIN_US,
            RX_WINDOW_MARGIN_US,
            self.ratio,
        );
        let (at, loops) = next_slot.ok_or(())?;
        let timeout = ADDR_DELAY_TICKS
            + PACKETBUF_AIRTIME_TICKS
            + TIMEOUT_MARGIN_TIMER_TICKS
            + RX_WINDOW_MARGIN_US * TIMER_TICKS_PER_US;
        self.timer.stop();
        self.timer.clear();
        self.timer.cc(2).write(0);
        self.timer.cc(1).write(timeout);
        self.ts.us = at;
        self.grtc.set_compare(at);
        Ok((loops, timeout))
    }

    fn read_sync(&self) -> u32 {
        self.timer.cc(2).read()
    }

    fn update_sync(&self, tm: &mut Timing) -> UpdateResult {
        if !tm.is_hb {
            return UpdateResult::Skipped;
        }
        let Some(sync) = self.read_sync().checked_sub(ADDR_DELAY_TICKS) else {
            return UpdateResult::Failed;
        };
        let Ok(sync_ticks) = i32::try_from(sync) else {
            return UpdateResult::Failed;
        };
        let drift = sync_ticks - (RX_WINDOW_MARGIN_US * TIMER_TICKS_PER_US) as i32;
        if drift.abs() > (DRIFT_LIMIT_US * TIMER_TICKS_PER_US) as i32 {
            return UpdateResult::Failed;
        }
        let Ok(hb) = self.update_timestamp_by_sync(self.ts, sync) else {
            return UpdateResult::Failed;
        };
        tm.set_drift(drift / TIMER_TICKS_PER_US as i32);
        tm.hb = hb;
        UpdateResult::Updated
    }

    fn update_timestamp_by_sync(&self, ts: Timestamp, sync: u32) -> Result<Timestamp, ()> {
        let us = ts
            .us
            .checked_add(self.ratio.hf_to_grtc(sync / TIMER_TICKS_PER_US).ok_or(())? as u64)
            .ok_or(())?;
        if us >= (1u64 << 52) {
            return Err(());
        }
        Ok(Timestamp { us })
    }
}
