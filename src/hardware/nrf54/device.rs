use crate::hardware::clock::{ClockDriver, GrtcChannel};
use crate::hardware::egu::YipEgu;
use crate::hardware::ppi::{
    Channel, Group as PpiGroup, PeriDomain, PpiDriver, Radio as RadioDomain,
};
use crate::hardware::radio::RadioDriver;
use crate::pdu::{PduByte0, PduByte1};
use crate::{config::*, register_egu};

use core::marker::PhantomData;
use embassy_nrf::ppib::BridgeChannel;
use embassy_nrf::{
    Peri,
    interrupt::typelevel::{Binding, EGU10, Handler},
    peripherals::RADIO,
    timer::Instance as TimerInstance,
};

#[derive(Default)]
pub(crate) struct DeviceContext {
    pub refslot: u32,
    pub slot_ticks: u32,
    pub pipes: usize,
}

impl DeviceContext {
    pub(crate) fn calculate_timeout_timer(&self, packet_len: usize) -> u32 {
        const HEADER_BITS: u32 = PduByte0::LENGTH_BIT_COUNT as u32 + PduByte1::S1_BITS as u32;

        const HEADER_TICKS: u32 = {
            assert!(RADIO_BITS_PER_US > 0);

            (HEADER_BITS * TIMER_TICKS_PER_US).div_ceil(RADIO_BITS_PER_US)
        };

        let my_tx_ticks =
            ADDR_DELAY_TICKS + HEADER_TICKS + (packet_len as u32 + 2) * TIMER_TICKS_PER_BYTE;
        let ur_tx_ticks = ADDR_DELAY_TICKS
            + HEADER_TICKS
            + (YIP_SUB_PACKET_SIZE as u32 + 2) * TIMER_TICKS_PER_BYTE;

        my_tx_ticks + ur_tx_ticks + TIMEOUT_MARGIN_TIMER_TICKS
    }
}

/// Hardware resources with a validated interrupt binding to handler `H`.
pub struct YipDevice<H> {
    pub(crate) radio: RadioDriver,
    pub(crate) clock: ClockDriver,
    pub(crate) ppi: PpiDriver,
    pub(crate) egu: YipEgu,
    pub(crate) context: DeviceContext,
    _handler: PhantomData<H>,
}

impl<H> YipDevice<H> {
    pub fn new<T, R, B>(
        radio: Peri<'static, RADIO>,
        timer: Peri<'static, T>,
        grtc: Peri<'static, R>,
        radio_channels: [Channel<'static, RadioDomain>; 3],
        peri_channels: [Channel<'static, PeriDomain>; 1],
        bridge: (Peri<'static, B>, Peri<'static, B::Paired>),
        group: PpiGroup<'static, RadioDomain>,
        _irq: impl Binding<EGU10, H>,
        seed: u32,
    ) -> Self
    where
        T: TimerInstance,
        R: GrtcChannel,
        B: BridgeChannel,
        H: Handler<EGU10>,
    {
        // we set refslot random here.
        let mut context = DeviceContext::default();
        context.refslot = seed;

        #[cfg(feature = "defmt")]
        defmt::debug!("seed: {}", seed);

        // we set egu register here for triggering from session.
        let egu = YipEgu::new();
        register_egu(egu.regs());

        let mut radio = RadioDriver::new(radio);
        radio.init();

        Self {
            radio: radio,
            clock: ClockDriver::new(grtc, timer),
            ppi: PpiDriver::new(radio_channels, peri_channels, bridge, group),
            egu: egu,
            context: context,
            _handler: PhantomData,
        }
    }
}

impl<H> YipDevice<H> {
    pub fn set_pipes(&mut self, pipes: usize) {
        assert!(pipes > 0, "pipes must be bigger than 0");

        self.context.pipes = pipes;
    }
}
