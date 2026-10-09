#![no_main]
#![no_std]

mod config;
mod device;
mod egu;
mod header;
mod hop;
mod isr;
mod link;
mod linkedlist;
mod packet;
mod packetbuffer;
mod pdu;
mod ppi;
mod protocol;
mod radio;
mod rng;
mod role;
mod session;
mod timing;

mod central;
mod peripheral;

pub use central::YipCentral;
pub use peripheral::YipPeripheral;

pub use device::YipDevice;
pub use isr::YipInterruptHandler;
pub use packet::{Packet, PacketChannels, PacketHandle, PacketPoolStorage, PacketPoolView};
pub use rng::random_u32;
pub use role::YipStack;
pub use session::{YipConnectionListener, YipReader, YipSession, YipWriter};

use embassy_nrf::pac;
use embassy_sync::once_lock::OnceLock;

use egu::{YipEgu, YipTrigger};
use role::YipRoleStack;

// hack
//

static OUTER_EGU: OnceLock<pac::egu::Egu> = OnceLock::new();

pub fn register_egu(regs: pac::egu::Egu) {
    let _ = OUTER_EGU.init(regs);
}

pub(crate) fn enable_interrupt() {
    let outer_egu = OUTER_EGU.try_get().expect("outer egu not initialized");
    YipEgu::enable_interrupt_regs(outer_egu, YipTrigger::Start);
}

pub(crate) fn trigger_start() {
    let outer_egu = OUTER_EGU.try_get().expect("outer egu not initialized");
    YipEgu::trigger_regs(outer_egu, YipTrigger::Start);
}

// Public only so exported macros can resolve their dependencies downstream.
#[doc(hidden)]
pub mod __private {
    pub use static_cell::StaticCell;
}

/// Initializes a stack for the selected role and returns one session per link.
/// `YipPeripheral` requires `links: 1`.
///
/// `pool_size` and `links` must be constant expressions. Initialization is
/// one-shot: calling the same expansion twice or registering a second stack
/// panics. Interrupt vector binding must be arranged separately.
#[macro_export]
macro_rules! init_yip {
    (
        device: $device:expr,
        pool_size: $pool_size:expr,
        links: $links:expr,
        role: $role:ty $(,)?
    ) => {{
        type Stack = $crate::YipStack<'static, 'static, { $links }, $role>;

        static POOL: $crate::__private::StaticCell<
            $crate::PacketPoolStorage<{ $pool_size }, { $links }>,
        > = $crate::__private::StaticCell::new();
        static VIEW: $crate::__private::StaticCell<$crate::PacketPoolView<'static, { $links }>> =
            $crate::__private::StaticCell::new();
        static CHANNELS: $crate::__private::StaticCell<
            $crate::PacketChannels<'static, { $links }>,
        > = $crate::__private::StaticCell::new();
        static STACK: $crate::__private::StaticCell<Stack> = $crate::__private::StaticCell::new();

        let pool: &'static mut $crate::PacketPoolStorage<{ $pool_size }, { $links }> =
            POOL.init($crate::PacketPoolStorage::new_packet());
        let view: &'static $crate::PacketPoolView<'static, { $links }> = VIEW.init(pool.view());
        let channels = CHANNELS.init($crate::PacketChannels::new(view));
        let (link_endpoints, session_endpoints) = channels.endpoints();

        let stack = STACK.init(Stack::new(link_endpoints, $device));
        stack.setup();
        stack.register_and_enable_irq();

        session_endpoints.map($crate::YipSession::new)
    }};
}
