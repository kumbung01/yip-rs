use crate::hardware::{
    clock::ClockDriver,
    egu::{EguDriver, YipEguInstance},
    ppi::{PpiChannel, PpiDriver, YIP_PPI_CHANNEL_COUNT},
    radio::YipRadio,
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

#[derive(Default)]
pub(crate) struct DeviceContext {
    pub refslot: u32,
    pub slot_ticks: u32,
    pub pipes: usize,
}

/// Hardware resources with a validated interrupt binding to handler `H`.
pub struct YipDevice<H> {
    pub(crate) radio: YipRadio,
    pub(crate) clock: ClockDriver,
    pub(crate) ppi: PpiDriver,
    pub(crate) egu: EguDriver,
    pub(crate) context: DeviceContext,
    _handler: PhantomData<H>,
}

impl<H> YipDevice<H> {
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
        E: YipEguInstance,
        G: Group,
        H: Handler<E::Interrupt>,
    {
        // we set refslot random here.
        let mut context = DeviceContext::default();
        context.refslot = seed;

        #[cfg(feature = "defmt")]
        defmt::debug!("seed: {}", seed);

        // we set egu register here for triggering from session.
        let egu = EguDriver::new(egu);
        register_egu(egu.regs());

        let mut radio = YipRadio::new(radio);
        radio.init();

        Self {
            radio: radio,
            clock: ClockDriver::new(rtc, timer),
            ppi: PpiDriver::new(channels, group.into()),
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
