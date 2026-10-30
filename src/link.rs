use crate::packet::*;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LinkState {
    Connected(LinkSubstate),
    Disconnected,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LinkSubstate {
    Normal,
}

pub struct Link {
    pub(crate) current_tx: Option<CurrentTx>,
    pub(crate) current_rx: Option<CurrentRx>,

    pub(crate) sn: bool,
    pub(crate) nesn: bool,

    pub(crate) tx_attempts: u32,

    pub(crate) state: LinkState,
}

impl Link {
    pub(crate) fn new() -> Self {
        Self {
            current_tx: None,
            current_rx: None,

            sn: false,
            nesn: false,

            tx_attempts: 0,

            state: LinkState::Disconnected,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.current_rx = None;
        self.current_tx = None;

        self.sn = false;
        self.nesn = false;

        self.tx_attempts = 0;

        self.state = LinkState::Disconnected;
    }
}
