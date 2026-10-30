#![no_main]
#![no_std]

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

use hardware::egu::{EguDriver, Trigger};

// hack
//

static OUTER_EGU: OnceLock<pac::egu::Egu> = OnceLock::new();

pub fn register_egu(regs: pac::egu::Egu) {
    let _ = OUTER_EGU.init(regs);
}

pub(crate) fn get_enabled() -> Option<Trigger> {
    debug!("enabled interrupt {}", &Trigger::Start);
    let outer_egu = OUTER_EGU.try_get().expect("outer egu not initialized");
    EguDriver::get_enabled_regs(outer_egu)
}

pub(crate) fn enable_start() {
    debug!("enabled interrupt {}", &Trigger::Start);
    let outer_egu = OUTER_EGU.try_get().expect("outer egu not initialized");
    EguDriver::enable_interrupt_regs(outer_egu, Trigger::Start);
}

pub(crate) fn trigger_start() {
    debug!("triggered interrupt {}", &Trigger::Start);
    let outer_egu = OUTER_EGU.try_get().expect("outer egu not initialized");
    EguDriver::trigger_regs(outer_egu, Trigger::Start);
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
        links: $links:expr,
        role: $role:ty,
        id: $id:expr $(,)?
    ) => {{
        type Stack = $crate::YipStack<{ $links }, $role>;

        static STACK: $crate::__private::StaticCell<Stack> = $crate::__private::StaticCell::new();

        let session_endpoints = channels.session_endpoints();

        let stack = STACK.init(Stack::new());
        stack.init($device, $id);

        session_endpoints.map($crate::YipSession::new)
    }};
}

use crate::peripheral::IRQ_STACK as PERIPHERAL_STACK;
use crate::peripheral::YipInterruptHandler as PeripheralIsr;
pub use crate::peripheral::YipPeripheral;
use embassy_nrf::interrupt::InterruptExt;

pub fn init_peripheral(device: YipDevice<PeripheralIsr>, id: u8) -> YipSession {
    hardware::cyccnt::init();

    let irq = device.egu.irq();

    let mut role = YipPeripheral::new(device, id);
    role.setup();

    PERIPHERAL_STACK.lock(|cell| {
        *cell.borrow_mut() = Some(role);
    });

    enable_start();

    unsafe {
        // SAFETY: 생성자의 Binding으로 벡터를 검증했고,
        // stack 초기화와 ISR 등록이 완료됨.
        irq.enable();
    }

    let [ep] = YipPeripheral::session_endpoints();
    YipSession::new(ep)
}

use crate::central::IRQ_STACK as CENTRAL_STACK;
use crate::central::YipInterruptHandler as CentralIsr;

pub fn init_central<const W: usize>(device: YipDevice<CentralIsr>) -> [YipSession; W] {
    hardware::cyccnt::init();

    let irq = device.egu.irq();

    CENTRAL_STACK.lock(|cell| {
        *cell.borrow_mut() = Some(YipCentral::new(device, W));
    });

    unsafe {
        // SAFETY: 생성자의 Binding으로 벡터를 검증했고,
        // stack 초기화와 ISR 등록이 완료됨.
        irq.enable();
    }

    let eps = YipCentral::session_endpoints();
    eps.map(|ep| YipSession::new(ep))
}
