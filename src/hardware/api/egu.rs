#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(usize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum Trigger {
    TimeslotEnd = 0,
    Start = 1,
}
impl Trigger {
    pub(crate) const ALL: [Self; 2] = [Self::TimeslotEnd, Self::Start];

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::TimeslotEnd),
            1 => Some(Self::Start),
            _ => None,
        }
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
    fn is_enabled_regs(regs: &Self::Registers, trigger: Trigger) -> bool;
    fn get_enabled_regs(regs: &Self::Registers) -> Option<Trigger>;
    fn enable_interrupt(&mut self, trigger: Trigger);
    fn disable_interrupt(&mut self, trigger: Trigger);
    fn select_interrupt(&mut self, trigger: Trigger);
    fn clear_triggered(&mut self, trigger: Trigger);
    fn trigger(&mut self, trigger: Trigger);
    fn is_triggered(&self, trigger: Trigger) -> bool;
    fn is_enabled(&self, trigger: Trigger) -> bool;
    fn get_enabled(&self) -> Option<Trigger>;
    fn get_and_clear_triggered(&mut self) -> Option<Trigger>;
    fn clear_all(&mut self);
}
