use crate::hardware::api::EguDriver;

use embassy_nrf::{
    Peri,
    egu::{Egu as EmbassyEgu, Instance},
    interrupt, pac,
};

use embassy_nrf::interrupt::typelevel::Interrupt as _;

pub(crate) struct Nrf52Egu {
    _egu: EmbassyEgu<'static>,
    regs: pac::egu::Egu,
    irq: interrupt::Interrupt,
}

impl Nrf52Egu {
    pub fn new<E: Nrf52EguInstance>(egu: Peri<'static, E>) -> Self {
        let regs = <E as Nrf52EguInstance>::regs();

        Self {
            _egu: EmbassyEgu::new(egu),
            irq: E::Interrupt::IRQ,
            regs,
        }
    }
}

pub(crate) use crate::hardware::api::egu::Trigger;

pub trait Nrf52EguInstance: Instance {
    fn regs() -> pac::egu::Egu;
}

macro_rules! impl_egu_regs {
    ($($reg:ident), +$(,)?) => {
        $(
            impl Nrf52EguInstance for embassy_nrf::peripherals::$reg {
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

impl EguDriver for Nrf52Egu {
    type Registers = pac::egu::Egu;

    fn regs(&self) -> pac::egu::Egu {
        self.regs
    }

    fn irq(&self) -> interrupt::Interrupt {
        self.irq
    }

    fn enable_interrupt_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.intenset()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    fn disable_interrupt_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.intenclr()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    fn select_interrupt_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.inten()
            .write(|w| w.set_triggered(trigger as usize, true));
    }

    fn clear_triggered_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.events_triggered(trigger as usize).write_value(0);
    }

    fn trigger_regs(regs: &pac::egu::Egu, trigger: Trigger) {
        regs.tasks_trigger(trigger as usize).write_value(1);
    }

    fn is_triggered_regs(regs: &pac::egu::Egu, trigger: Trigger) -> bool {
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
