use core::sync::atomic::{AtomicBool, Ordering};

use embassy_nrf::{
    interrupt,
    pac::{self, egu},
};

use embassy_nrf::interrupt::typelevel::Interrupt as _;

#[allow(dead_code)]
pub(crate) struct EguDriver {
    regs: egu::Egu,
    irq: interrupt::Interrupt,
}

// Embassy does not expose an EGU10 ownership token on this chip.
static EGU10_TAKEN: AtomicBool = AtomicBool::new(false);

pub(crate) use EguDriver as YipEgu;

#[allow(dead_code)]
impl EguDriver {
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

    pub fn regs(&self) -> egu::Egu {
        self.regs
    }

    pub(crate) fn irq(&self) -> interrupt::Interrupt {
        self.irq
    }

    pub(crate) fn enable_interrupt_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.intenset()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn disable_interrupt_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.intenclr()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn select_interrupt_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.inten()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn clear_triggered_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.events_triggered(trigger as usize).write_value(0);
    }

    pub(crate) fn trigger_regs(regs: &egu::Egu, trigger: Trigger) {
        regs.tasks_trigger(trigger as usize).write_value(1);
    }

    pub(crate) fn is_triggered_regs(regs: &egu::Egu, trigger: Trigger) -> bool {
        regs.events_triggered(trigger as usize).read() != 0
    }

    pub(crate) fn is_enabled_regs(regs: &egu::Egu, trigger: Trigger) -> bool {
        regs.intenset().read().triggered(trigger as usize)
    }

    pub(crate) fn get_enabled_regs(regs: &egu::Egu) -> Option<Trigger> {
        let flags = regs.intenset().read().0;
        Trigger::from_index(flags.trailing_zeros() as usize)
    }

    pub fn enable_interrupt(&mut self, trigger: Trigger) {
        Self::enable_interrupt_regs(&self.regs(), trigger);
    }

    pub fn disable_interrupt(&mut self, trigger: Trigger) {
        Self::disable_interrupt_regs(&self.regs(), trigger);
    }

    pub fn select_interrupt(&mut self, trigger: Trigger) {
        Self::select_interrupt_regs(&self.regs(), trigger);
    }

    pub fn clear_triggered(&mut self, trigger: Trigger) {
        Self::clear_triggered_regs(&self.regs(), trigger);
    }

    pub fn trigger(&mut self, trigger: Trigger) {
        Self::trigger_regs(&self.regs(), trigger);
    }

    pub fn is_triggered(&self, trigger: Trigger) -> bool {
        Self::is_triggered_regs(&self.regs(), trigger)
    }

    pub fn is_enabled(&self, trigger: Trigger) -> bool {
        Self::is_enabled_regs(&self.regs(), trigger)
    }

    pub fn get_enabled(&self) -> Option<Trigger> {
        let flags = self.regs.intenset().read().0;
        Trigger::from_index(flags.trailing_zeros() as usize)
    }

    pub fn get_and_clear_triggered(&mut self) -> Option<Trigger> {
        let trigger = self
            .get_enabled()
            .filter(|&trigger| self.is_triggered(trigger))?;

        self.disable_interrupt(trigger);
        self.clear_triggered(trigger);

        Some(trigger)
    }

    pub fn clear_all(&mut self) {
        for trigger in Trigger::ALL {
            self.disable_interrupt(trigger);
            self.clear_triggered(trigger);
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(usize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum Trigger {
    TimeslotEnd = 0,
    Start = 1,
}

impl Trigger {
    const ALL: [Self; 2] = [Self::TimeslotEnd, Self::Start];

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::TimeslotEnd),
            1 => Some(Self::Start),
            _ => None,
        }
    }
}

// EGU10 is the fixed interrupt source for the nRF54 backend.
macro_rules! impl_yip_egu_handlers {
    () => {
        $crate::isr::impl_yip_egu_handlers!(EGU10);
    };
}

pub(crate) use impl_yip_egu_handlers;
