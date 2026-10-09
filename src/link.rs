use crate::config::*;
use crate::packet::{CurrentPacket, YipLinkEndpoints};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LinkState {
    Connected,
    Disconnected,
}

pub(crate) struct YipLink<'q, 'p, const W: usize> {
    pub(crate) endpoints: YipLinkEndpoints<'q, 'p, W>,

    pub(crate) current_tx: Option<CurrentPacket<'p, W>>,
    pub(crate) current_rx: Option<CurrentPacket<'p, W>>,

    pub(crate) sn: bool,
    pub(crate) nesn: bool,

    pub(crate) last_rx_crc: u32,
    pub(crate) tx_attempts: u16,
    pub(crate) rssi: RssiState,
    pub(crate) tx_power: TxPowerState,

    pub(crate) state: LinkState,
    pub(crate) last_sync: u32,
}

impl<'q, 'p, const W: usize> YipLink<'q, 'p, W> {
    pub(crate) fn new(endpoints: YipLinkEndpoints<'q, 'p, W>) -> Self {
        Self {
            endpoints: endpoints,

            current_tx: None,
            current_rx: None,

            sn: false,
            nesn: false,

            last_rx_crc: 0,
            tx_attempts: 0,

            rssi: RssiState::new(),
            tx_power: TxPowerState::new(),

            state: LinkState::Disconnected,
            last_sync: 0,
        }
    }

    pub(crate) fn check_timeout_central(&mut self, timer_now: u32) -> LinkState {
        let in_sync = timer_now.wrapping_sub(self.last_sync)
            <= ms_to_timer_ticks(TIMEOUT_LIMIT_MS_CENTRAL) as u32;

        self.state = if in_sync {
            LinkState::Connected
        } else {
            LinkState::Disconnected
        };

        self.state
    }

    pub(crate) fn update_link_state(&mut self, now: u32) {
        let changed = self.state == LinkState::Disconnected;

        self.state = LinkState::Connected;
        self.last_sync = now;

        if changed {
            self.endpoints.conn.signal(self.state);
        }
    }

    pub(crate) fn reset(&mut self) {
        self.current_rx = None;
        self.current_tx = None;

        self.sn = false;
        self.nesn = false;

        self.last_rx_crc = 0;
        self.tx_attempts = 0;

        self.rssi.reset();
        self.tx_power.reset();

        self.last_sync = 0;
        self.state = LinkState::Disconnected;
    }
}

use fixed::{FixedU32, types::extra::U10};
type UScaled = FixedU32<U10>;

pub(crate) struct RssiState {
    value: UScaled,
}

impl RssiState {
    fn new() -> Self {
        Self {
            value: UScaled::from_num(RSSI_BASELINE),
        }
    }

    pub(crate) fn get(&self) -> u8 {
        self.value.to_num::<u8>()
    }

    pub(crate) fn reset(&mut self) {
        self.value = UScaled::from_num(RSSI_BASELINE);
    }

    pub(crate) fn update(&mut self, sample: u8) {
        let sample = UScaled::from_num(sample);
        let alpha = UScaled::from_num(120) / UScaled::from_num(1024);
        self.value = alpha * sample + (UScaled::from_num(1) - alpha) * self.value;
    }
}

pub(crate) struct TxPowerState {
    index: usize,
    settle_count: u8,
    direction: i8,
}

impl TxPowerState {
    fn new() -> Self {
        Self {
            index: TX_POWER_INITIAL_INDEX,
            settle_count: 0,
            direction: 0,
        }
    }

    fn get(&self) -> i8 {
        TX_POWER_LEVELS[self.index]
    }

    fn reset(&mut self) {
        self.index = TX_POWER_INITIAL_INDEX;
        self.settle_count = 0;
        self.direction = 0;
    }

    fn update(&mut self, rssi: u8) -> bool {
        let diff = i16::from(rssi) - i16::from(RSSI_BASELINE);

        let direction: i8 = if diff > RSSI_DIFF_THRESHOLD_UP {
            1
        } else if diff < RSSI_DIFF_THRESHOLD_DN {
            -1
        } else {
            self.settle_count = 0;
            self.direction = 0;
            return false;
        };

        if direction != self.direction {
            self.settle_count = 0;
            self.direction = direction;
        }

        self.settle_count = self.settle_count.saturating_add(1);

        if self.settle_count < TX_POWER_SAMPLE_CNT {
            return false;
        }

        self.settle_count = 0;

        let next = if direction > 0 {
            (self.index + 1).min(TX_POWER_LEVELS.len() - 1)
        } else {
            self.index.saturating_sub(1)
        };

        let changed = next != self.index;
        self.index = next;
        changed
    }
}
