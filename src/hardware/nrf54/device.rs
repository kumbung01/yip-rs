use crate::hardware::api::{EguDriver, RadioDriver};
use crate::hardware::clock::{GrtcChannel, Nrf54Clock};
use crate::hardware::context::SlotContext;
use crate::hardware::egu::Nrf54Egu;
use crate::hardware::ppi::{
    Channel, Group as PpiGroup, Nrf54Ppi, PeriDomain, RADIO_CHANNEL_COUNT, Radio as RadioDomain,
};
use crate::hardware::radio::Nrf54Radio;
use crate::register_egu;

use crate::hardware::ppi::BridgeChannel;
use core::marker::PhantomData;
use embassy_nrf::{
    Peri,
    interrupt::typelevel::{Binding, EGU10, Handler},
    peripherals::RADIO,
};

/// Hardware resources with a validated interrupt binding to handler `H`.
pub struct DeviceContext<H> {
    pub(crate) radio: Nrf54Radio,
    pub(crate) clock: Nrf54Clock,
    pub(crate) ppi: Nrf54Ppi,
    pub(crate) egu: Nrf54Egu,
    pub(crate) context: SlotContext,
    _handler: PhantomData<H>,
}

impl<H> DeviceContext<H> {
    pub fn new<R, B>(
        radio: Peri<'static, RADIO>,
        timer: Peri<'static, embassy_nrf::peripherals::TIMER10>,
        grtc: Peri<'static, R>,
        radio_channels: [Channel<'static, RadioDomain>; RADIO_CHANNEL_COUNT],
        peri_channels: [Channel<'static, PeriDomain>; 1],
        bridge: (Peri<'static, B>, Peri<'static, B::Paired>),
        groups: [PpiGroup<'static, RadioDomain>; 4],
        _irq: impl Binding<EGU10, H>,
        seed: u32,
    ) -> Self
    where
        R: GrtcChannel,
        B: BridgeChannel,
        H: Handler<EGU10>,
    {
        let context = SlotContext {
            refslot: seed,
            ..SlotContext::default()
        };

        #[cfg(feature = "defmt")]
        defmt::debug!("seed: {}", seed);

        // we set egu register here for triggering from session.
        let egu = Nrf54Egu::new();
        register_egu(egu.regs());

        let mut radio = Nrf54Radio::new(radio);
        radio.init();

        Self {
            radio,
            clock: Nrf54Clock::new(grtc, timer),
            ppi: Nrf54Ppi::new(radio_channels, peri_channels, bridge, groups),
            egu,
            context,
            _handler: PhantomData,
        }
    }
}

impl<H> DeviceContext<H> {
    pub fn set_pipes(&mut self, pipes: usize) {
        assert!(pipes > 0, "pipes must be bigger than 0");

        self.context.pipes = pipes;
    }
}
