#[derive(Clone, Copy, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum Trigger {
    TimeslotEnd,
    Start,
    #[cfg(feature = "nrf54lm20")]
    Rx(Rx),
    #[cfg(feature = "nrf54lm20")]
    PrepareRx(PrepareRx),
    #[cfg(feature = "nrf54lm20")]
    TxAck(TxAck),
}

macro_rules! state_triggers {
    ($($state:ident { $($name:ident = $number:literal),+ $(,)? })+) => {
        $(
            #[cfg(feature = "nrf54lm20")]
            #[derive(Clone, Copy, Eq, PartialEq)]
            #[repr(usize)]
            #[cfg_attr(feature = "defmt", derive(defmt::Format))]
            pub(crate) enum $state { $($name = $number),+ }
        )+
    };
}

// Reserve low indices for role interrupts; allocate DPPI relays from the end.
state_triggers! {
    Rx { Rxen = 15, Failure = 14, Done = 13, Disabled = 12 }
    PrepareRx { Wake = 11 }
    TxAck { Disabled = 10, Done = 9, Start = 8 }
}

impl Trigger {
    pub(crate) const ALL: &'static [Self] = &[
        Self::TimeslotEnd,
        Self::Start,
        #[cfg(feature = "nrf54lm20")]
        Self::Rx(Rx::Rxen),
        #[cfg(feature = "nrf54lm20")]
        Self::PrepareRx(PrepareRx::Wake),
        #[cfg(feature = "nrf54lm20")]
        Self::Rx(Rx::Failure),
        #[cfg(feature = "nrf54lm20")]
        Self::Rx(Rx::Done),
        #[cfg(feature = "nrf54lm20")]
        Self::Rx(Rx::Disabled),
        #[cfg(feature = "nrf54lm20")]
        Self::TxAck(TxAck::Disabled),
        #[cfg(feature = "nrf54lm20")]
        Self::TxAck(TxAck::Done),
        #[cfg(feature = "nrf54lm20")]
        Self::TxAck(TxAck::Start),
    ];

    pub(crate) const fn number(self) -> usize {
        match self {
            Self::TimeslotEnd => 0,
            Self::Start => 1,
            #[cfg(feature = "nrf54lm20")]
            Self::Rx(trigger) => trigger as usize,
            #[cfg(feature = "nrf54lm20")]
            Self::PrepareRx(trigger) => trigger as usize,
            #[cfg(feature = "nrf54lm20")]
            Self::TxAck(trigger) => trigger as usize,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|trigger| trigger.number() == index)
    }
}

/// Required hardware API. Every chip backend must implement all methods.
pub(crate) trait EguDriver {
    type Registers;

    fn regs(&self) -> Self::Registers;
    fn irq(&self) -> embassy_nrf::interrupt::Interrupt;
    fn enable_interrupt_regs(regs: &Self::Registers, trigger: Trigger);
    fn disable_interrupt_regs(regs: &Self::Registers, trigger: Trigger);
    fn select_interrupt_regs(regs: &Self::Registers, trigger: Trigger);
    fn clear_triggered_regs(regs: &Self::Registers, trigger: Trigger);
    fn trigger_regs(regs: &Self::Registers, trigger: Trigger);
    fn is_triggered_regs(regs: &Self::Registers, trigger: Trigger) -> bool;

    fn disable_interrupt(&mut self, trigger: Trigger);
    fn select_interrupt(&mut self, trigger: Trigger);
    fn clear_triggered(&mut self, trigger: Trigger);

    fn is_triggered(&self, trigger: Trigger) -> bool;

    fn get_enabled(&self) -> Option<Trigger>;
    fn get_and_clear_triggered(&mut self) -> Option<Trigger>;
    fn clear_all(&mut self);
}
