use core::marker::PhantomData;
use core::ops::Sub;
use embassy_time::Instant;

use embassy_nrf::{
    Peri, PeripheralType,
    pac::GRTC,
    peripherals,
    timer::{Instance as TimerInstance, Timer},
};

use crate::pdu::*;
use crate::{config::*, hardware::cyccnt};

/*
 * Clock handles all timing related functions.
 */

pub(crate) trait GrtcChannel: PeripheralType + 'static {
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

// GRTC_CH11 is reserved by Embassy's time-driver-grtc feature.
impl_grtc_channels!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10);

struct GrtcCc<'d> {
    number: usize,
    _p: PhantomData<&'d ()>,
}

impl<'d> GrtcCc<'d> {
    pub fn new<G: GrtcChannel>(_token: Peri<'d, G>) -> Self {
        debug!("grtc: {}", G::NUMBER);
        Self {
            number: G::NUMBER,
            _p: PhantomData,
        }
    }

    pub fn set_compare(&mut self, timestamp: u64) {
        let cc = embassy_nrf::pac::GRTC.cc(self.number);

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

impl Timestamp {
    pub fn unscaled(&self) -> Timestamp {
        Timestamp { us: self.us }
    }
}

#[derive(Default, Clone, Copy)]
pub(crate) struct SlotClock {
    pub(crate) slot: u32,
    pub(crate) timestamp: u64,
}

pub(crate) struct ClockDriver {
    grtc: GrtcCc<'static>,
    pub(crate) timer: Timer<'static>,
    ts: Timestamp,
    slot: u32,
}

pub(crate) struct SlotDiff(u32);

impl ClockDriver {
    pub fn new<G: GrtcChannel, T: TimerInstance>(
        grtc: Peri<'static, G>,
        timer: Peri<'static, T>,
    ) -> Self {
        let mut timer = Timer::new(timer);

        timer.set_frequency(TIMER_FREQUENCY);
        // timer.regs().prescaler().write(|w| w.set_prescaler(5));
        timer.regs().bitmode().write(|w| {
            w.set_bitmode(TIMER_BITMODE);
        });

        Self {
            grtc: GrtcCc::new(grtc),
            timer: timer,
            ts: Timestamp { us: 0 },
            slot: 0,
        }
    }

    pub(crate) fn get_start_ts(&self) -> Timestamp {
        self.ts
    }

    pub(crate) fn get_start_hw(&mut self) -> Timestamp {
        Timestamp {
            us: Instant::now().as_micros(),
        }
    }
}

// hw specific impl
impl ClockDriver {
    pub(crate) fn grtc_num(&self) -> usize {
        self.grtc.number
    }
}

// central impl
impl ClockDriver {
    pub(crate) fn get_timeout(&self) -> u32 {
        self.timer.cc(0).read()
    }

    pub(crate) fn start_central(&mut self) {
        // self.timer.cc(0).short_compare_stop();
        // self.timer.cc(0).short_compare_clear();
    }

    pub(crate) fn stop_central(&mut self) {
        self.timer.stop();
        self.timer.clear();
    }

    pub(crate) fn set_next_slot_central(&mut self) -> u32 {
        let now_us = Instant::now().as_micros();
        let elapsed_us = (now_us - self.ts.us) as u32;
        let radio_margin_us = ISR_MARGIN_US;
        let slot_diff = (elapsed_us + radio_margin_us).div_ceil(YIP_SLOTSIZE_US as u32);
        let tx_start = self
            .ts
            .us
            .wrapping_add((slot_diff).wrapping_mul(YIP_SLOTSIZE_US as u32) as u64);

        self.ts.us = tx_start;

        self.grtc.set_compare(tx_start);

        slot_diff
    }

    pub(crate) fn set_timeout_central(&mut self, bytes: usize) -> u32 {
        const HEADER_BITS: u32 = PduByte0::LENGTH_BIT_COUNT as u32 + PduByte1::S1_BITS as u32;

        const HEADER_TICKS: u32 = {
            assert!(RADIO_BITS_PER_US > 0);

            (HEADER_BITS * TIMER_TICKS_PER_US).div_ceil(RADIO_BITS_PER_US)
        };

        let tx_ticks = ADDR_DELAY_TICKS
            + HEADER_TICKS
            + (bytes as u32 + RADIO_CRC_LEN_BYTES) * TIMER_TICKS_PER_BYTE;
        let rx_ticks = ADDR_DELAY_TICKS
            + HEADER_TICKS
            + (YIP_SUB_PACKET_SIZE as u32 + RADIO_CRC_LEN_BYTES) * TIMER_TICKS_PER_BYTE;

        let timeout_ticks = tx_ticks + rx_ticks + TIMEOUT_MARGIN_TIMER_TICKS;
        assert!(
            timeout_ticks < us_to_timer_ticks(YIP_SLOTSIZE_US as u32 - TIMESLOT_MARGIN_US) as u32,
            "timetout_ticks: {}, slotsize timeout: {}",
            timeout_ticks,
            us_to_timer_ticks(YIP_SLOTSIZE_US as u32 - TIMESLOT_MARGIN_US) as u32
        );

        self.timer.cc(0).write(timeout_ticks);

        timeout_ticks
    }

    pub(crate) fn get_now_central(&self) -> Timestamp {
        let us = Instant::now().as_micros();

        Timestamp { us: us }
    }

    pub(crate) fn get_start_central(&self) -> Timestamp {
        self.ts
    }
}

#[derive(Default)]
pub(crate) struct Timing {
    pub hb: Timestamp,
    pub drift_scaled: Option<i32>,
    pub hb_loops: u32,
    pub loop_ticks_scaled: u32,
    pub slot_now: u32,

    pub is_hb: bool,
    pub check_next_slot: bool,
}

impl Timing {
    fn set_drift(&mut self, drift: i32) {
        let error_scaled = drift * TIMER_TICK_SCALED as i32;

        if let Some(ds) = &mut self.drift_scaled {
            *ds += error_scaled / 4;
        } else {
            self.drift_scaled = Some(error_scaled);
        }
    }

    fn get_drift_scaled(&self) -> i32 {
        self.drift_scaled.unwrap_or(0)
    }

    pub fn drift_reset(&mut self) {
        self.drift_scaled = None;
    }
}

// peripheral impl
impl ClockDriver {
    pub(crate) fn start_peripheral(&mut self) {
        todo!();
    }

    pub(crate) fn stop_peripheral(&mut self) {
        todo!();
    }

    pub(crate) fn get_now_peripheral(&self) -> u64 {
        todo!();
    }

    pub(crate) fn set_next_sync(&mut self) -> (u32, u32) {
        todo!();
    }

    pub(crate) fn set_next_pre_slot(&mut self, tm: &Timing) {
        todo!();
    }

    pub(crate) fn set_next_slot_peripheral(&mut self, tm: &Timing) -> Result<(u32, u32), ()> {
        todo!();
    }

    pub(crate) fn read_sync(&self) -> u32 {
        todo!();
    }

    pub(crate) fn update_sync(&self, tm: &mut Timing) -> UpdateResult {
        todo!();
    }

    pub(crate) fn update_timestamp_by_sync(ts: Timestamp, sync: u32) -> Result<Timestamp, ()> {
        todo!();
    }
}

fn unscale_timer(cc: u32) -> u32 {
    todo!();
}
