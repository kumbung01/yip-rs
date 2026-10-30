use cortex_m::peripheral::{DWT, Peripherals};

pub fn init() {
    let mut core = Peripherals::take().expect("Core peripherals already taken");

    core.DCB.enable_trace();
    core.DWT.set_cycle_count(0);
    core.DWT.enable_cycle_counter();
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Cycle(pub u32);

impl Cycle {
    pub fn as_cycles(&self) -> u32 {
        self.0
    }

    pub fn as_us(&self) -> u32 {
        self.0 / 64
    }
}

pub fn start() -> Cycle {
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    let cycles = DWT::cycle_count();
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    Cycle(cycles)
}

pub fn end(start: Cycle) -> Cycle {
    Cycle(DWT::cycle_count().wrapping_sub(start.as_cycles()))
}
