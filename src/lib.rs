#![no_main]
#![no_std]

#[cfg(all(feature = "defmt", feature = "log"))]
compile_error!("YIP logging features defmt and log are mutually exclusive");

use crate::hardware::api::EguDriver;
#[macro_use]
mod config;
mod hardware;
mod hop;
mod isr;
pub(crate) mod link;
mod linkedlist;
mod packet;
mod packetbuffer;
pub(crate) mod payload;
mod pdu;
mod role;
mod session;
mod signal;

pub mod central;
pub mod peripheral;

pub use central::YipCentral;
pub use hardware::device::*;
#[doc(hidden)]
pub use packet::{Packet, PacketHandle};
pub use session::{YipConnectionListener, YipReader, YipSession, YipWriter};

use embassy_nrf::pac;
use embassy_sync::once_lock::OnceLock;

use hardware::Egu;
use hardware::egu::Trigger;

// hack
//

static OUTER_EGU: OnceLock<pac::egu::Egu> = OnceLock::new();

pub(crate) fn register_egu(regs: pac::egu::Egu) {
    let _ = OUTER_EGU.init(regs);
}

pub(crate) fn enable_start() {
    debug!("enabled interrupt {:?}", &Trigger::Start);
    let outer_egu = OUTER_EGU.try_get().expect("outer egu not initialized");
    Egu::enable_interrupt_regs(outer_egu, Trigger::Start);
}

pub(crate) fn trigger_start() {
    debug!("triggered interrupt {:?}", &Trigger::Start);
    let outer_egu = OUTER_EGU.try_get().expect("outer egu not initialized");
    Egu::trigger_regs(outer_egu, Trigger::Start);
}

// Public only so exported macros can resolve their dependencies downstream.
#[doc(hidden)]
pub mod __private {
    pub use static_cell::StaticCell;
}
