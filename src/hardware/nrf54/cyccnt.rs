use cortex_m::peripheral::{DWT, Peripherals};

pub fn init() {
    let mut core = Peripherals::take().expect("Core peripherals already taken");

    core.DCB.enable_trace();
    core.DWT.set_cycle_count(0);
    core.DWT.enable_cycle_counter();

    #[cfg(feature = "defmt")]
    defmt::debug!("CPU clock: {} MHz", cycles_per_us());
    #[cfg(feature = "defmt")]
    {
        use embassy_nrf::pac::{ICACHE, POWER, RRAMC};
        defmt::debug!(
            "ACK memory config: cache_enabled {} rram_lowpower_mode {} rram_active_us {} constlat_raw {}",
            ICACHE.enable().read().enable(),
            RRAMC.power().lowpowerconfig().read().mode().to_bits(),
            RRAMC.power().config().read().accesstimeout() as u32 / 32,
            POWER.constlatstat().read().0
        );
    }
}

fn cycles_per_us() -> u32 {
    use embassy_nrf::pac::{OSCILLATORS, oscillators::vals::Currentfreq};

    match OSCILLATORS.pll().currentfreq().read().currentfreq() {
        Currentfreq::Ck64m => 64,
        Currentfreq::Ck128m => 128,
        _ => unreachable!("unsupported CPU clock"),
    }
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Cycle(pub u32);

impl Cycle {
    pub fn as_cycles(&self) -> u32 {
        self.0
    }

    pub fn as_us(&self) -> u32 {
        self.0 / cycles_per_us()
    }
}

pub fn start() -> Cycle {
    // Bound measured RAM work as well as volatile peripheral accesses. These
    // are compiler barriers; they do not issue hardware synchronization tasks.
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    let cycles = DWT::cycle_count();
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    Cycle(cycles)
}

pub fn end(start: Cycle) -> Cycle {
    Cycle(DWT::cycle_count().wrapping_sub(start.as_cycles()))
}
