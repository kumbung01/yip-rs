use embassy_nrf::interrupt;

use core::cell::RefCell;
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};

pub(crate) trait InterruptTarget: Send {
    fn on_interrupt(&mut self);
}

static IRQ_STACK: Mutex<
    CriticalSectionRawMutex,
    RefCell<Option<&'static mut dyn InterruptTarget>>,
> = Mutex::new(RefCell::new(None));

pub struct YipInterruptHandler;

pub(crate) fn register(stack: &'static mut dyn InterruptTarget) {
    IRQ_STACK.lock(|slot| {
        let mut slot = slot.borrow_mut();

        assert!(slot.is_none(), "YIP stack already registered");
        *slot = Some(stack);
    });
}

fn dispatch() {
    IRQ_STACK.lock(|slot| {
        let mut slot = slot.borrow_mut();

        if let Some(stack) = slot.as_deref_mut() {
            stack.on_interrupt();
        }
    });
}

macro_rules! impl_yip_egu_handlers {
    ($($irq:ident),+ $(,)?) => {
        $(
            impl interrupt::typelevel::Handler<
                interrupt::typelevel::$irq,
            > for YipInterruptHandler {
                unsafe fn on_interrupt() {
                    dispatch();
                }
            }
        )+
    };
}

impl_yip_egu_handlers!(
    EGU0_SWI0, EGU1_SWI1, EGU2_SWI2, EGU3_SWI3, EGU4_SWI4, EGU5_SWI5,
);
