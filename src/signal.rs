use embassy_nrf::pac::radio::vals::Txpower;

use crate::config::*;
use crate::hardware::radio::{TX_POWER, TX_POWER_INITIAL_INDEX};

pub(crate) struct TxpowerState {
    index: usize,
    settle_count: u8,
    direction: i8,
}

impl TxpowerState {
    pub(crate) fn new() -> Self {
        Self {
            index: TX_POWER_INITIAL_INDEX,
            settle_count: 0,
            direction: 0,
        }
    }

    pub(crate) fn get(&self) -> Txpower {
        TX_POWER[self.index]
    }

    pub(crate) fn reset(&mut self) {
        self.index = TX_POWER_INITIAL_INDEX;
        self.settle_count = 0;
        self.direction = 0;
    }

    pub(crate) fn update(&mut self, rssi: u8) -> bool {
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
            (self.index + 1).min(TX_POWER.len() - 1)
        } else {
            self.index.saturating_sub(1)
        };

        let changed = next != self.index;
        self.index = next;
        changed
    }

    pub(crate) fn raise(&mut self) {
        self.settle_count = 0;
        self.direction = 1;
        self.index = (self.index + 1).min(TX_POWER.len() - 1)
    }
}

use fixed::{FixedU32, types::extra::U10};
type UScaled = FixedU32<U10>;

pub struct RssiState {
    value: UScaled,
}

impl RssiState {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn update(&mut self, sample: u8) -> bool {
        let sample = UScaled::from_num(sample);
        let alpha = UScaled::from_num(120) / UScaled::from_num(1024);
        self.value = alpha * sample + (UScaled::from_num(1) - alpha) * self.value;

        true
    }
}
