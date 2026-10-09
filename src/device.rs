use crate::egu::{YipEgu, YipEguInstance};
use crate::isr::YipInterruptHandler;
use crate::pdu::{PduByte0, PduByte1};
use crate::ppi::{PpiChannel, YIP_PPI_CHANNEL_COUNT, YipPpi};
use crate::radio::YipRadio;
use crate::{config::*, register_egu};

use crate::timing::SlotClock;
use embassy_nrf::{
    Peri,
    interrupt::typelevel::{Binding, Handler},
    peripherals::RADIO,
    ppi::Group,
    rtc::{Instance as RtcInstance, Rtc},
    timer::{Frequency, Instance as TimerInstance, Timer},
};

#[derive(Default)]
pub(crate) struct DeviceContext {
    pub start: u32,
    pub refslot: u32,
    pub slotsize_ticks: u32,
    pub pipes: u8,
    pub clock: SlotClock,
}

impl DeviceContext {
    pub(crate) fn calculate_timeout_timer(&self, packet_len: usize) -> u32 {
        const HEADER_TICKS: u32 =
            (PduByte0::LENGTH_BIT_COUNT as u32 + PduByte1::S1_BITS as u32) * TIMER_TICKS_PER_BIT;

        let my_tx_ticks =
            ADDR_DELAY_TICKS + HEADER_TICKS + (packet_len as u32 + 2) * TIMER_TICKS_PER_BYTE;
        let ur_tx_ticks = ADDR_DELAY_TICKS
            + HEADER_TICKS
            + (YIP_SUB_PACKET_SIZE as u32 + 2) * TIMER_TICKS_PER_BYTE;

        my_tx_ticks + ur_tx_ticks + TIMEOUT_MARGIN_TIMER_TICKS
    }

    pub(crate) const fn set_slotsize(&mut self) {
        self.slotsize_ticks = SLOTSIZE_TIMER_TICKS;
    }
}

pub struct YipDevice {
    pub(crate) radio: YipRadio,
    pub(crate) timer: Timer<'static>,
    pub(crate) rtc: Rtc<'static>,
    pub(crate) ppi: YipPpi<'static>,
    pub(crate) egu: YipEgu,
    pub(crate) context: DeviceContext,
}

impl YipDevice {
    pub fn new<T, R, E, G>(
        radio: Peri<'static, RADIO>,
        timer: Peri<'static, T>,
        rtc: Peri<'static, R>,
        channels: [PpiChannel<'static>; YIP_PPI_CHANNEL_COUNT],
        group: Peri<'static, G>,
        egu: Peri<'static, E>,
        _irq: impl Binding<E::Interrupt, YipInterruptHandler>,
        seed: u32,
    ) -> Self
    where
        T: TimerInstance,
        R: RtcInstance,
        E: YipEguInstance,
        G: Group,
        YipInterruptHandler: Handler<E::Interrupt>,
    {
        // we set refslot random here.
        let mut context = DeviceContext::default();
        context.refslot = seed;

        defmt::debug!("seed: {}", seed);

        // we set egu register here for triggering from session.
        let egu = YipEgu::new(egu);
        register_egu(egu.regs());

        let rtc = Rtc::new(rtc, RTC_PRESCALE).expect("RTC init failed");
        let mut timer = Timer::new(timer);
        timer.set_frequency(TIMER_FREQUENCY);
        timer.regs().bitmode().write(|w| {
            w.set_bitmode(TIMER_BITMODE);
        });

        Self {
            radio: YipRadio::new(radio),
            timer: timer,
            rtc: rtc,
            ppi: YipPpi::new(channels, group.into()),
            egu: egu,
            context: context,
        }
    }
}

impl YipDevice {
    pub fn set_pipes(&mut self, pipes: u8) {
        assert!(pipes > 0, "pipes must be bigger than 0");

        self.context.pipes = pipes;
    }
}
