use crate::hardware::api::EguDriver;

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_nrf::{
    interrupt,
    pac::{self, egu},
};

use embassy_nrf::interrupt::typelevel::Interrupt as _;

pub(crate) struct Nrf54Egu {
    regs: egu::Egu,
    irq: interrupt::Interrupt,
}

// Embassy does not expose an EGU10 ownership token on this chip.
static EGU10_TAKEN: AtomicBool = AtomicBool::new(false);

impl Nrf54Egu {
    pub fn new() -> Self {
        assert!(
            !EGU10_TAKEN.swap(true, Ordering::Relaxed),
            "EGU10 is already in use"
        );

        Self {
            irq: interrupt::typelevel::EGU10::IRQ,
            regs: pac::EGU10,
        }
    }
}

pub(crate) use crate::hardware::api::egu::Trigger;

// EGU10 is the fixed interrupt source for the nRF54 backend.
macro_rules! impl_yip_egu_handlers {
    () => {
        $crate::isr::impl_yip_egu_handlers!(EGU10);
    };
}

pub(crate) use impl_yip_egu_handlers;

impl EguDriver for Nrf54Egu {
    type Registers = egu::Egu;

    fn regs(&self) -> egu::Egu {
        self.regs
    }

    fn irq(&self) -> interrupt::Interrupt {
        self.irq
    }

    fn enable_interrupt_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.intenset()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    fn disable_interrupt_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.intenclr()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    fn select_interrupt_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.inten()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    fn clear_triggered_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.events_triggered(trigger as usize).write_value(0);
    }

    fn trigger_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.tasks_trigger(trigger as usize).write_value(1);
    }

    fn is_triggered_regs(regs: &egu::Egu, trigger: Trigger) -> bool {
        regs.events_triggered(trigger as usize).read() != 0
    }

    fn disable_interrupt(&mut self, trigger: Trigger) {
        Self::disable_interrupt_regs(&self.regs(), trigger);
    }

    fn select_interrupt(&mut self, trigger: Trigger) {
        Self::select_interrupt_regs(&self.regs(), trigger);
    }

    fn clear_triggered(&mut self, trigger: Trigger) {
        Self::clear_triggered_regs(&self.regs(), trigger);
    }

    fn is_triggered(&self, trigger: Trigger) -> bool {
        Self::is_triggered_regs(&self.regs(), trigger)
    }

    fn get_enabled(&self) -> Option<Trigger> {
        let flags = self.regs.intenset().read().0;
        Trigger::from_index(flags.trailing_zeros() as usize)
    }

    fn get_and_clear_triggered(&mut self) -> Option<Trigger> {
        let trigger = self
            .get_enabled()
            .filter(|&trigger| self.is_triggered(trigger))?;

        self.disable_interrupt(trigger);
        self.clear_triggered(trigger);

        Some(trigger)
    }

    fn clear_all(&mut self) {
        for trigger in Trigger::ALL {
            self.disable_interrupt(trigger);
            self.clear_triggered(trigger);
        }
    }
}
