use core::mem::size_of;
use embassy_nrf::pac::{radio::vals as radiovals, timer::vals as timervals};
use embassy_nrf::timer::Frequency;

use crate::config::RadioBitrate::*;
use crate::packetbuffer::PacketBuffer;

#[derive(PartialEq, Eq)]
pub(crate) enum UpdateResult {
    Skipped,
    Updated,
    Failed,
}

// Yip limits
pub(crate) const YIP_MAX_LINK_COUNT: usize = 7;
pub(crate) const YIP_POOL_SIZE: usize = 8;

// Yip configs
pub(crate) const YIP_SLOTSIZE_US: u64 = 500;
pub(crate) const YIP_PACKET_SIZE: usize = 251;
pub(crate) const YIP_SUB_PACKET_SIZE: usize = 20;
pub(crate) const CHANNELS: u32 = 39; // 39 channels
// 250 us over the 500 ms heartbeat interval corresponds to 500 ppm.
pub(crate) const DRIFT_LIMIT_US: u32 = 250;

// timeslot constants
pub(crate) const ISR_MARGIN_US: u32 = 20;
pub(crate) const HEARTBEAT_DURATION_US: u32 = 500_000;
pub(crate) const PERIPHEARL_SYNC_DURATION_US: u32 = 750_000;
pub(crate) const PERIPHERAL_SYNC_TIMETOUT_US: u32 = 250_000;

pub(crate) const TIMEOUT_MARGIN_US: u32 = 20;
pub(crate) const TIMESLOT_MARGIN_US: u32 = 20;
pub(crate) const RX_WINDOW_MARGIN_US: u32 = 20;

pub(crate) const TIMEOUT_LIMIT_US_CENTRAL: u32 = 1000_000;
pub(crate) const TIMEOUT_LIMIT_US_PERIPHERAL: u32 = 500_000;

// radio configs
pub enum RadioBitrate {
    _1mbit,
    _2mbit,
    #[cfg(feature = "nrf54lm20")]
    _4mbit,
}

#[allow(dead_code)] // Both ramp-up modes remain valid configuration choices.
pub enum RadioRampup {
    Normal,
    Fast,
}

pub(crate) const YIP_RADIO_BITRATE: RadioBitrate = RadioBitrate::_2mbit;
pub(crate) const YIP_RADIO_RAMPUP_MODE: RadioRampup = RadioRampup::Fast;
pub(crate) const RADIO_PREAMBLE_LEN_MODE: radiovals::Plen = radiovals::Plen::_16bit;

pub(crate) const RADIO_CRC_LEN_MODE: radiovals::Len = radiovals::Len::Two;
pub(crate) const RADIO_CRC_SKIPADDR: radiovals::Skipaddr = radiovals::Skipaddr::Skip;
pub(crate) const RADIO_TXADDRESS_INVALID: u8 = 7;
pub(crate) const RADIO_RSSI_PADDING_BYTES: usize = 4;

// base addresses + prefixes per pipe
pub(crate) const RADIO_ADDR_PREFIX_0: [u8; 4] = [0xE7, 0xC2, 0xC3, 0xC4];
pub(crate) const RADIO_ADDR_PREFIX_1: [u8; 4] = [0xC5, 0xC6, 0xC7, 0xC8];
pub(crate) const RADIO_ADDR_BASE_0: [u8; 4] = [0xE7, 0xE7, 0xE7, 0xE7];
pub(crate) const RADIO_ADDR_BASE_1: [u8; 4] = [0xC2, 0xC2, 0xC2, 0xC2];

// scale between timer and rtc.
#[cfg(feature = "nrf52840")]
pub(crate) const TIMER_TICK_SCALED: u32 = 32 * (1 << TIMER_PRESCALE);

// timer configs
pub(crate) const TIMER_BITMODE: timervals::Bitmode = timervals::Bitmode::_32bit;
pub(crate) const TIMER_FREQUENCY: Frequency = Frequency::F1MHz;
#[cfg(feature = "nrf52840")]
pub(crate) const TIMER_PRESCALE: u32 = TIMER_FREQUENCY as u32;

// rtc configs
#[cfg(feature = "nrf52840")]
pub(crate) const RTC_PRESCALE: u32 = 0;

// rssi configs
pub(crate) const RSSI_BASELINE: u8 = 60;
pub(crate) const RSSI_DIFF_THRESHOLD_UP: i16 = 4;
pub(crate) const RSSI_DIFF_THRESHOLD_DN: i16 = -2;

pub(crate) const TX_POWER_INITIAL_DB: radiovals::Txpower = radiovals::Txpower::_0dBm;
pub(crate) const TX_POWER_SAMPLE_CNT: u8 = 8;

// constants for peripherals

// RTC CC has to be set at least 3 ticks before triggered.
#[cfg(feature = "nrf52840")]
const RTC_COMPARE_MARGIN_TICKS: u32 = 3;

// radio derived constants
#[cfg(feature = "nrf52840")]
pub(crate) const RADIO_MARGIN_RTC_TICKS: u32 = {
    let ticks = us_to_rtc_ticks(ISR_MARGIN_US);
    if ticks < RTC_COMPARE_MARGIN_TICKS as u64 {
        RTC_COMPARE_MARGIN_TICKS
    } else {
        ticks as u32
    }
};

pub(crate) const RADIO_CRC_LEN_BYTES: u32 = match RADIO_CRC_LEN_MODE {
    radiovals::Len::Disabled => 0,
    radiovals::Len::One => 1,
    radiovals::Len::Two => 2,
    radiovals::Len::Three => 3,
};

pub(crate) const RADIO_RAMPUP_US: u32 = match YIP_RADIO_RAMPUP_MODE {
    RadioRampup::Normal => 129,
    RadioRampup::Fast => 40,
};

#[cfg(feature = "nrf52840")]
pub(crate) const RTC_TICK_HZ: u64 = {
    assert!(RTC_PRESCALE <= 4095);
    assert!(32_768 % (RTC_PRESCALE as u64 + 1) == 0);

    32_768 / (RTC_PRESCALE as u64 + 1)
};

pub(crate) const TIMER_TICK_HZ: u64 = {
    // Frequency encodes log2 prescaling from 16 MHz. Negative values select
    // the faster nRF54 clocks. This also works with released embassy-nrf 0.11,
    // which does not yet expose Frequency::as_hz().
    let prescaler = TIMER_FREQUENCY as u8 as i8;
    if prescaler >= 0 {
        16_000_000u64 >> prescaler as u32
    } else {
        16_000_000u64 << (-prescaler) as u32
    }
};
pub(crate) const TIMER_TICKS_PER_US: u32 = {
    assert!(TIMER_TICK_HZ % 1_000_000 == 0);

    (TIMER_TICK_HZ / 1_000_000) as u32
};

pub(crate) const TIMEOUT_MARGIN_TIMER_TICKS: u32 = TIMEOUT_MARGIN_US * TIMER_TICKS_PER_US;

pub(crate) const RADIO_BITS_PER_US: u32 = match YIP_RADIO_BITRATE {
    _1mbit => 1,
    _2mbit => 2,
    #[cfg(feature = "nrf54lm20")]
    _4mbit => 4,
};

pub(crate) const TIMER_TICKS_PER_BYTE: u32 = {
    assert!(RADIO_BITS_PER_US > 0);
    assert!((8 * TIMER_TICKS_PER_US) % RADIO_BITS_PER_US == 0);

    8 * TIMER_TICKS_PER_US / RADIO_BITS_PER_US
};

// pub(crate) const YIP_RADIO_RAMPUP_MODE: radiovals::Ru = radiovals::Ru::Default;
pub(crate) const RADIO_RAMPUP_TIMER_TICKS: u32 = RADIO_RAMPUP_US * TIMER_TICKS_PER_US;

pub(crate) const RADIO_ADDR_BALEN: u8 = RADIO_ADDR_BASE_0.len() as u8;
pub(crate) const RADIO_ADDR_BYTES: u8 = RADIO_ADDR_BALEN + 1; // BASE_ADDR[4] + PREFIX[1]
pub(crate) const RADIO_ADDR_BITS: u32 = RADIO_ADDR_BYTES as u32 * 8;

pub(crate) const RADIO_PREAMBLE_BYTES: u8 = match RADIO_PREAMBLE_LEN_MODE {
    radiovals::Plen::_8bit => 1,
    radiovals::Plen::_16bit => 2,
    _ => panic!("unsupported preamble mode"),
};
pub(crate) const RADIO_PREAMBLE_BITS: u32 = RADIO_PREAMBLE_BYTES as u32 * 8;

pub(crate) const ADDR_DELAY_TICKS: u32 = RADIO_RAMPUP_TIMER_TICKS
    + (RADIO_PREAMBLE_BITS + RADIO_ADDR_BITS) * TIMER_TICKS_PER_US / RADIO_BITS_PER_US;

pub(crate) const PACKETBUF_AIRTIME_TICKS: u32 =
    (size_of::<PacketBuffer>() as u32 + RADIO_CRC_LEN_BYTES) * TIMER_TICKS_PER_BYTE;

// RTC_TICK_HZ는 실제 RTC 주파수 상수 이름에 맞춰 사용
#[cfg(feature = "nrf52840")]
pub(crate) const RTC_TICK_SCALED: u64 = {
    let scaled_timer_hz = TIMER_TICKS_PER_US as u64 * 1_000_000 * TIMER_TICK_SCALED as u64;

    assert!(RTC_TICK_HZ > 0);
    assert!(scaled_timer_hz % RTC_TICK_HZ == 0);

    scaled_timer_hz / RTC_TICK_HZ
};

#[cfg(feature = "nrf52840")]
pub(crate) const RTC_MARGIN_SCALED: u32 = RADIO_MARGIN_RTC_TICKS * RTC_TICK_SCALED as u32;

// helper functions
#[inline]
#[cfg(feature = "nrf52840")]
pub(crate) const fn us_to_rtc_ticks(us: u32) -> u64 {
    (us as u64 * RTC_TICK_HZ).div_ceil(1_000_000)
}

pub(crate) const fn us_to_timer_ticks(us: u32) -> u64 {
    us as u64 * TIMER_TICKS_PER_US as u64
}

#[cfg(feature = "nrf52840")]
pub(crate) const RTC_MASK: u32 = 0x00FF_FFFF;

#[inline]
#[cfg(feature = "nrf52840")]
pub(crate) const fn rtc_wrap(value: u32) -> u32 {
    value & RTC_MASK
}

#[inline]
#[cfg(feature = "nrf52840")]
pub(crate) const fn rtc_elapsed(now: u32, since: u32) -> u32 {
    rtc_wrap(now.wrapping_sub(since))
}

pub(crate) const fn index_of(values: &[radiovals::Txpower], target: radiovals::Txpower) -> usize {
    let mut i = 0;
    while i < values.len() {
        if values[i].to_bits() == target.to_bits() {
            return i;
        }
        i += 1;
    }
    panic!("value not found");
}

macro_rules! debug {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::debug!($($arg)*);
        #[cfg(feature = "log")]
        log::debug!($($arg)*);
    };
}

macro_rules! error {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::error!($($arg)*);
        #[cfg(feature = "log")]
        log::error!($($arg)*);
    };
}

macro_rules! info {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::info!($($arg)*);
        #[cfg(feature = "log")]
        log::info!($($arg)*);
    };
}
