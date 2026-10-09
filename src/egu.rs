use embassy_nrf::{
    Peri,
    egu::{Egu, Instance},
    interrupt, pac,
};

use embassy_nrf::interrupt::typelevel::Interrupt as _;

#[allow(dead_code)]
pub(crate) struct YipEgu {
    egu: Egu<'static>,
    regs: pac::egu::Egu,
    irq: interrupt::Interrupt,
}

#[allow(dead_code)]
impl YipEgu {
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

    pub(crate) fn enable_interrupt_regs(regs: &pac::egu::Egu, trigger: YipTrigger) {
        regs.intenset()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn disable_interrupt_regs(regs: &pac::egu::Egu, trigger: YipTrigger) {
        regs.intenclr()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn select_interrupt_regs(regs: &pac::egu::Egu, trigger: YipTrigger) {
        regs.inten()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    pub(crate) fn clear_triggered_regs(regs: &pac::egu::Egu, trigger: YipTrigger) {
        regs.events_triggered(trigger as usize).write_value(0);
    }

    pub(crate) fn trigger_regs(regs: &pac::egu::Egu, trigger: YipTrigger) {
        regs.tasks_trigger(trigger as usize).write_value(1);
    }

    pub(crate) fn is_triggered_regs(regs: &pac::egu::Egu, trigger: YipTrigger) -> bool {
        regs.events_triggered(trigger as usize).read() != 0
    }

    pub(crate) fn is_enabled_regs(regs: &pac::egu::Egu, trigger: YipTrigger) -> bool {
        regs.intenset().read().triggered(trigger as usize)
    }

    pub(crate) fn get_enabled_regs(regs: &pac::egu::Egu) -> Option<YipTrigger> {
        let flags = regs.intenset().read().0;
        YipTrigger::from_index(flags.trailing_zeros() as usize)
    }

    pub fn enable_interrupt(&mut self, trigger: YipTrigger) {
        Self::enable_interrupt_regs(&self.regs(), trigger);
    }

    pub fn disable_interrupt(&mut self, trigger: YipTrigger) {
        Self::disable_interrupt_regs(&self.regs(), trigger);
    }

    pub fn select_interrupt(&mut self, trigger: YipTrigger) {
        Self::select_interrupt_regs(&self.regs(), trigger);
    }

    pub fn clear_triggered(&mut self, trigger: YipTrigger) {
        Self::clear_triggered_regs(&self.regs(), trigger);
    }

    pub fn trigger(&mut self, trigger: YipTrigger) {
        Self::trigger_regs(&self.regs(), trigger);
    }

    pub fn is_triggered(&self, trigger: YipTrigger) -> bool {
        Self::is_triggered_regs(&self.regs(), trigger)
    }

    pub fn is_enabled(&self, trigger: YipTrigger) -> bool {
        Self::is_enabled_regs(&self.regs(), trigger)
    }

    pub fn get_enabled(&self) -> Option<YipTrigger> {
        let flags = self.regs.intenset().read().0;
        YipTrigger::from_index(flags.trailing_zeros() as usize)
    }

    //     pub(crate) fn take_interrupt_events(&mut self) -> EguEvents {
    //         let mut events = EguEvents::default();
    //
    //         if self.is_triggered(YipTrigger::TimeslotEnd) {
    //             self.clear_triggered(YipTrigger::TimeslotEnd);
    //             self.disable_interrupt(YipTrigger::TimeslotEnd);
    //             events.timeslot_end = true;
    //         }
    //
    //         if self.is_triggered(YipTrigger::Start) {
    //             self.clear_triggered(YipTrigger::Start);
    //             self.disable_interrupt(YipTrigger::Start);
    //             events.start = true;
    //         }
    //
    //         if self.is_triggered(YipTrigger::Disabled) {
    //             self.clear_triggered(YipTrigger::Disabled);
    //             self.disable_interrupt(YipTrigger::Disabled);
    //             events.disabled = true;
    //         }
    //
    //         events
    //     }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(usize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum YipTrigger {
    TimeslotEnd = 0,
    Start = 1,
    Disabled = 2,
}
//
// #[derive(Default)]
// pub(crate) struct EguEvents {
//     pub(crate) timeslot_end: bool,
//     pub(crate) start: bool,
//     pub(crate) disabled: bool,
// }

impl YipTrigger {
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
