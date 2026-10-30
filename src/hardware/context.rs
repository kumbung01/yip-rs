/// Scheduling state shared by both hardware backends.
#[derive(Default)]
pub(crate) struct SlotContext {
    pub refslot: u32,
    pub slot_ticks: u32,
    pub pipes: usize,
}
