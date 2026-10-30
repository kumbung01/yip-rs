use embassy_nrf::{
    Peri,
    egu::{Egu, Instance},
    interrupt, pac,
};

use embassy_nrf::interrupt::typelevel::Interrupt as _;

#[allow(dead_code)]
pub(crate) struct EguDriver {
    egu: Egu<'static>,
    regs: pac::egu::Egu,
    irq: interrupt::Interrupt,
}

#[allow(dead_code)]
impl EguDriver {
    pub fn new<E: YipEguInstance>(egu: Peri<'static, E>) -> Self {
        let regs = <E as YipEguInstance>::regs();

        Self {
            egu: Egu::new(egu),
            irq: E::Interrupt::IRQ,
            regs,
        }
    }

    pub fn regs(&self) -> pac::egu::Egu {
        self.regs
    }

    pub(crate) fn irq(&self) -> interrupt::Interrupt {
        self.irq
    }

    pub(crate) fn enable_interrupt_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.intenset()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn disable_interrupt_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.intenclr()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn select_interrupt_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.inten()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn clear_triggered_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.events_triggered(trigger as usize).write_value(0);
    }

    pub(crate) fn trigger_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.tasks_trigger(trigger as usize).write_value(1);
    }

    pub(crate) fn is_triggered_regs(regs: &pac::egu::Egu, trigger: Trigger) -> bool {
        regs.events_triggered(trigger as usize).read() != 0
    }

    pub(crate) fn is_enabled_regs(regs: &pac::egu::Egu, trigger: Trigger) -> bool {
        regs.intenset().read().triggered(trigger as usize)
    }

    pub(crate) fn get_enabled_regs(regs: &pac::egu::Egu) -> Option<Trigger> {
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
//
// #[derive(Default)]
// pub(crate) struct EguEvents {
//     pub(crate) timeslot_end: bool,
//     pub(crate) start: bool,
//     pub(crate) disabled: bool,
// }

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

pub trait YipEguInstance: Instance {
    fn regs() -> pac::egu::Egu;
}

macro_rules! impl_egu_regs {
    ($($reg:ident), +$(,)?) => {
        $(
            impl YipEguInstance for embassy_nrf::peripherals::$reg {
                fn regs() -> pac::egu::Egu {
                    pac::$reg
                }
            }
        )+
    };
}

impl_egu_regs!(EGU0, EGU1, EGU2, EGU3, EGU4, EGU5);

// Keep the chip's IRQ names out of the common role implementations.
macro_rules! impl_yip_egu_handlers {
    () => {
        $crate::isr::impl_yip_egu_handlers!(
            EGU0_SWI0, EGU1_SWI1, EGU2_SWI2, EGU3_SWI3, EGU4_SWI4, EGU5_SWI5,
        );
    };
}

pub(crate) use impl_yip_egu_handlers;
