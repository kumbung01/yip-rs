use crate::hardware::api::{EguDriver, RadioDriver};
use crate::hardware::context::SlotContext;
use crate::hardware::{
    clock::Nrf52Clock,
    egu::{Nrf52Egu, Nrf52EguInstance},
    ppi::{Nrf52Ppi, PpiChannel, YIP_PPI_CHANNEL_COUNT},
    radio::Nrf52Radio,
};
use crate::register_egu;

use core::marker::PhantomData;
use embassy_nrf::{
    Peri,
    interrupt::typelevel::{Binding, Handler},
    peripherals::RADIO,
    ppi::Group,
    rtc::Instance as RtcInstance,
    timer::Instance as TimerInstance,
};

/// Hardware resources with a validated interrupt binding to handler `H`.
pub struct DeviceContext<H> {
    pub(crate) radio: Nrf52Radio,
    pub(crate) clock: Nrf52Clock,
    pub(crate) ppi: Nrf52Ppi,
    pub(crate) egu: Nrf52Egu,
    pub(crate) context: SlotContext,
    _handler: PhantomData<H>,
}

impl<H> DeviceContext<H> {
    /// Reserve a second PPI group for one-shot central RX completion.
    pub fn with_central_completion_group<G: Group>(mut self, group: Peri<'static, G>) -> Self {
        self.ppi.set_central_completion_group(group.into());
        self
    }

    pub fn new<T, R, E, G>(
        radio: Peri<'static, RADIO>,
        timer: Peri<'static, T>,
        rtc: Peri<'static, R>,
        channels: [PpiChannel<'static>; YIP_PPI_CHANNEL_COUNT],
        group: Peri<'static, G>,
        egu: Peri<'static, E>,
        _irq: impl Binding<E::Interrupt, H>,
        seed: u32,
    ) -> Self
    where
        T: TimerInstance,
        R: RtcInstance,
        E: Nrf52EguInstance,
        G: Group,
        H: Handler<E::Interrupt>,
    {
        let context = SlotContext {
            refslot: seed,
            ..SlotContext::default()
        };

        #[cfg(any(feature = "defmt", feature = "log"))]
        debug!("seed: {}", seed);

        // we set egu register here for triggering from session.
        let egu = Nrf52Egu::new(egu);
        register_egu(egu.regs());

        let mut radio = Nrf52Radio::new(radio);
        radio.init();

        let mut device = Self {
            radio,
            clock: Nrf52Clock::new(rtc, timer),
            ppi: Nrf52Ppi::new(channels, group.into()),
            egu,
            context,
            _handler: PhantomData,
        };
        #[cfg(any(feature = "defmt", feature = "log"))]
        device.ppi.measure_rtc_ratio(&mut device.clock);
        device
    }
}

impl<H> DeviceContext<H> {
    pub fn set_pipes(&mut self, pipes: usize) {
        assert!(pipes > 0, "pipes must be bigger than 0");

        self.context.pipes = pipes;
    }
}
