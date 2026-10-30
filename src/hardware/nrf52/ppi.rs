use crate::hardware::api::{EguDriver, PpiDriver};

use embassy_nrf::{
    Peri,
    pac::{
        self,
        common::{RW, Reg, W},
        ppi::regs::Chen,
    },
    ppi::{AnyConfigurableChannel, AnyGroup, Channel, Group},
};

use crate::hardware::clock::Nrf52Clock;
use crate::hardware::egu::{Nrf52Egu, Trigger};

pub(crate) const YIP_PPI_CHANNEL_COUNT: usize = 8;
pub(crate) type PpiChannel<'d> = Peri<'d, AnyConfigurableChannel>;
pub(crate) type PpiGroup<'d> = Peri<'d, AnyGroup>;

#[macro_export]
macro_rules! ppi_channels {
    ($($channel:expr),+ $(,)?) => {
        [$($channel.into()),+]
    };
}

pub(crate) struct Nrf52Ppi {
    channels: [PpiChannel<'static>; YIP_PPI_CHANNEL_COUNT],
    mask: Chen,
    group: PpiGroup<'static>,
}

#[allow(dead_code)]
impl Nrf52Ppi {
    pub(crate) fn new(
        channels: [PpiChannel<'static>; YIP_PPI_CHANNEL_COUNT],
        group: PpiGroup<'static>,
    ) -> Self {
        let mask = Chen(
            channels
                .iter()
                .fold(0u32, |mask, ch| mask | (1u32 << ch.number())),
        );
        let mut this = Self {
            channels,
            mask,
            group: group.into(),
        };
        this.clear_all();
        this
    }

    fn include_group(&mut self, channel: usize) {
        pac::PPI.chg(self.group.number()).modify(|w| {
            w.set_ch(self.channels[channel].number(), true);
        })
    }

    fn remove(&mut self, channel: usize) {
        pac::PPI.chg(self.group.number()).write(|w| {
            w.set_ch(self.channels[channel].number(), false);
        })
    }

    fn connect(
        &mut self,
        channel: usize,
        event: Reg<u32, RW>,
        task0: Reg<u32, W>,
        task1: Option<Reg<u32, W>>,
    ) -> u32 {
        let n = self.channels[channel].number();
        let ppi = pac::PPI;

        ppi.ch(n).eep().write_value(event.as_ptr() as u32);
        ppi.ch(n).tep().write_value(task0.as_ptr() as u32);
        ppi.fork(n)
            .tep()
            .write_value(task1.map_or(0, |reg| reg.as_ptr() as u32));

        1u32 << self.channels[channel].number()
    }

    fn clear_all(&mut self) {
        let ppi = embassy_nrf::pac::PPI;

        self.clear();

        // 기존 endpoint 설정을 제거한다.
        for channel in &self.channels {
            let number = channel.number();

            ppi.ch(number).eep().write_value(0);
            ppi.ch(number).tep().write_value(0);
            ppi.fork(number).tep().write_value(0);
        }
    }

    fn enable_channels(&self, channels: u32) {
        pac::PPI.chenset().write_value(Chen(channels));
    }
}

impl Drop for Nrf52Ppi {
    fn drop(&mut self) {
        self.clear_all();
    }
}

impl PpiDriver for Nrf52Ppi {
    type Clock = Nrf52Clock;
    type Egu = Nrf52Egu;

    fn clear(&mut self) {
        let ppi = embassy_nrf::pac::PPI;

        // 우선 채널을 모두 끈다.
        self.disable();

        // 그룹에서 모든 채널을 제거한다.
        ppi.chg(self.group.number()).write(|_| ());
    }

    fn enable(&mut self) {
        pac::PPI.chenset().write_value(self.mask);
    }

    fn disable(&mut self) {
        pac::PPI.chenclr().write_value(self.mask);
    }

    fn configure_central(&mut self, clock: &mut Nrf52Clock, _egu: &mut Nrf52Egu) {
        let ppi = pac::PPI;
        let radio = pac::RADIO;
        let timer = clock.timer.regs();
        let egu = _egu.regs();

        let timeslot_end = Trigger::TimeslotEnd as usize;

        let group_num = self.group.number();
        let task_group_enable = ppi.tasks_chg(group_num).en();
        let task_group_disable = ppi.tasks_chg(group_num).dis();

        // timer.comp0 -> radio.txen, group.en
        self.connect(
            0,
            timer.events_compare(0),
            radio.tasks_txen(),
            Some(task_group_enable),
        );
        self.include_group(0);

        // radio.disabled -> (radio.rxen, group.dis)
        self.connect(
            1,
            radio.events_disabled(),
            radio.tasks_rxen(),
            Some(task_group_disable),
        );
        self.include_group(1);

        // (timer.comp1 | radio.crcok) -> (egu.trigger0 -> SWI)
        self.connect(
            2,
            timer.events_compare(1),
            egu.tasks_trigger(timeslot_end),
            Some(task_group_disable),
        );

        self.connect(
            3,
            radio.events_crcok(),
            egu.tasks_trigger(timeslot_end),
            None,
        );

        // egu.triggered0 -> (radio.disable)
        self.connect(
            4,
            egu.events_triggered(timeslot_end),
            radio.tasks_disable(),
            None,
        );

        // configure_central()
        self.connect(5, radio.events_end(), timer.tasks_capture(2), None);
        self.include_group(5);
    }

    fn configure_sync(&mut self, _clock: &mut Nrf52Clock, _egu: &mut Nrf52Egu) {
        let radio = pac::RADIO;
        let timer = _clock.timer.regs();
        let rtc = _clock.rtc.regs();
        let egu = _egu.regs();

        self.clear();

        // rtc.comp0 -> timer.en
        let mut channels = self.connect(0, rtc.events_compare(0), timer.tasks_start(), None);

        // timer.comp0 -> radio.rxen
        channels |= self.connect(1, timer.events_compare(0), radio.tasks_rxen(), None);

        // radio.addr -> timer.cap2,
        channels |= self.connect(2, radio.events_address(), timer.tasks_capture(2), None);

        // timer.comp1 || radio.end(short) -> radio.dis
        channels |= self.connect(3, timer.events_compare(1), radio.tasks_disable(), None);

        // radio.disabled -> egu.trig(TimeslotEnd), timer.shutdown
        channels |= self.connect(
            4,
            radio.events_disabled(),
            egu.tasks_trigger(Trigger::TimeslotEnd as usize),
            Some(timer.tasks_shutdown()),
        );

        self.enable_channels(channels);
    }

    fn configure_peripheral_prepare_rx(&mut self, _clock: &mut Nrf52Clock, _egu: &mut Nrf52Egu) {
        let rtc = _clock.rtc.regs();
        let egu = _egu.regs();

        self.clear();

        let channels = self.connect(
            0,
            rtc.events_compare(0),
            egu.tasks_trigger(Trigger::Start as usize),
            None,
        );

        self.enable_channels(channels);
    }

    fn configure_peripheral_next_rx(&mut self, _clock: &mut Nrf52Clock, _egu: &mut Nrf52Egu) {
        let ppi = pac::PPI;
        let radio = pac::RADIO;
        let timer = _clock.timer.regs();
        let rtc = _clock.rtc.regs();
        let egu = _egu.regs();

        let timeslot_end = Trigger::TimeslotEnd as usize;

        let group_num = self.group.number();
        let task_group_enable = ppi.tasks_chg(group_num).en();
        let task_group_disable = ppi.tasks_chg(group_num).dis();

        self.clear();

        // rtc.comp0 -> (timer.start, group.en)
        let mut channels = self.connect(
            0,
            rtc.events_compare(0),
            timer.tasks_start(),
            Some(task_group_enable),
        );

        channels |= self.connect(1, timer.events_compare(0), radio.tasks_rxen(), None);
        self.include_group(1);

        // radio.rxaddr -> timer.capture2
        channels |= self.connect(2, radio.events_address(), timer.tasks_capture(2), None);
        self.include_group(2);

        // (timer.comp1 || radio.crcerror) -> (egu.trigger(timeslot_end) -> SWI, group.dis)
        channels |= self.connect(
            3,
            timer.events_compare(1),
            egu.tasks_trigger(timeslot_end),
            Some(task_group_disable),
        );
        channels |= self.connect(
            4,
            radio.events_crcerror(),
            egu.tasks_trigger(timeslot_end),
            Some(task_group_disable),
        );

        // radio.crcok -> egu.trigger(timeslot_end)
        channels |= self.connect(
            5,
            radio.events_crcok(),
            egu.tasks_trigger(timeslot_end),
            None,
        );

        // egu.triggered(timeslot_end) -> radio.disable, timer.shutdown
        channels |= self.connect(
            6,
            egu.events_triggered(timeslot_end),
            radio.tasks_disable(),
            Some(timer.tasks_shutdown()),
        );

        // radio.disabled -> (radio.txen, group.dis)
        channels |= self.connect(
            7,
            radio.events_disabled(),
            radio.tasks_txen(),
            Some(task_group_disable),
        );
        self.include_group(7);

        self.enable_channels(channels);
    }

    fn configure_peripheral_tx_ack(&mut self, _egu: &mut Nrf52Egu) {
        let radio = pac::RADIO;
        let egu = _egu.regs();

        self.clear();

        let channels = self.connect(
            0,
            radio.events_disabled(),
            egu.tasks_trigger(Trigger::TimeslotEnd as usize),
            None,
        );

        self.enable_channels(channels);
    }
}
