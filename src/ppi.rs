use embassy_nrf::{
    Peri,
    pac::{
        self,
        common::{RW, Reg, W},
        ppi::regs::Chen,
    },
    ppi::{AnyConfigurableChannel, AnyGroup, Channel, Group},
    rtc::Rtc,
    timer::Timer,
};

use crate::egu::{YipEgu, YipTrigger};
use crate::radio::YipRadio;

pub(crate) const YIP_PPI_CHANNEL_COUNT: usize = 8;
pub(crate) type PpiChannel<'d> = Peri<'d, AnyConfigurableChannel>;
pub(crate) type PpiGroup<'d> = Peri<'d, AnyGroup>;

#[macro_export]
macro_rules! ppi_channels {
    ($($channel:expr),+ $(,)?) => {
        [$($channel.into()),+]
    };
}

pub(crate) struct YipPpi<'d> {
    channels: [PpiChannel<'d>; YIP_PPI_CHANNEL_COUNT],
    mask: Chen,
    group: PpiGroup<'d>,
}

impl<'d> YipPpi<'d> {
    pub(crate) fn new(
        channels: [PpiChannel<'d>; YIP_PPI_CHANNEL_COUNT],
        group: PpiGroup<'d>,
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
        this.clear();
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

    fn clear(&mut self) {
        let ppi = embassy_nrf::pac::PPI;

        // 우선 채널을 모두 끈다.
        self.disable();

        // 그룹에서 모든 채널을 제거한다.
        ppi.chg(self.group.number()).write(|_| ());

        // 기존 endpoint 설정을 제거한다.
        for channel in &self.channels {
            let number = channel.number();

            ppi.ch(number).eep().write_value(0);
            ppi.ch(number).tep().write_value(0);
            ppi.fork(number).tep().write_value(0);
        }
    }

    pub(crate) fn configure_central(
        &mut self,
        _radio: &mut YipRadio,
        _timer: &mut Timer<'static>,
        _egu: &mut YipEgu,
    ) {
        let ppi = pac::PPI;
        let radio = _radio.regs();
        let timer = _timer.regs();
        let egu = _egu.regs();

        let timeslot_end = YipTrigger::TimeslotEnd as usize;

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

    pub(crate) fn configure_sync(
        &mut self,
        _radio: &mut YipRadio,
        _timer: &mut Timer<'static>,
        _rtc: &mut Rtc<'static>,
        _egu: &mut YipEgu,
    ) {
        let radio = _radio.regs();
        let timer = _timer.regs();
        let rtc = _rtc.regs();
        let egu = _egu.regs();

        self.disable();

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
            egu.tasks_trigger(YipTrigger::TimeslotEnd as usize),
            Some(timer.tasks_shutdown()),
        );

        self.enable_channels(channels);
    }

    pub(crate) fn configure_peripheral_prepare_rx(
        &mut self,
        _rtc: &mut Rtc<'static>,
        _egu: &mut YipEgu,
    ) {
        let rtc = _rtc.regs();
        let egu = _egu.regs();

        self.disable();

        let channels = self.connect(
            0,
            rtc.events_compare(0),
            egu.tasks_trigger(YipTrigger::Start as usize),
            None,
        );

        self.enable_channels(channels);
    }

    pub(crate) fn configure_peripheral_next_rx(
        &mut self,
        _radio: &mut YipRadio,
        _timer: &mut Timer<'static>,
        _rtc: &mut Rtc<'static>,
        _egu: &mut YipEgu,
    ) {
        let ppi = pac::PPI;
        let radio = _radio.regs();
        let timer = _timer.regs();
        let rtc = _rtc.regs();
        let egu = _egu.regs();

        let timeslot_end = YipTrigger::TimeslotEnd as usize;
        let timeout = YipTrigger::Disabled as usize;

        let group_num = self.group.number();
        let task_group_enable = ppi.tasks_chg(group_num).en();
        let task_group_disable = ppi.tasks_chg(group_num).dis();

        let mut channels: u32 = 0;

        self.disable();

        // rtc.comp0 -> (timer.start, radio.rxen, group.en)
        channels |= self.connect(
            0,
            rtc.events_compare(0),
            timer.tasks_start(),
            Some(radio.tasks_rxen()),
        );
        channels |= self.connect(1, rtc.events_compare(0), task_group_enable, None);

        // radio.rxaddr -> timer.capture2,
        // include group
        channels |= self.connect(2, radio.events_address(), timer.tasks_capture(2), None);
        self.include_group(2);

        // radio.disabled -> (radio.txen, group.dis)
        // include group
        channels |= self.connect(
            3,
            radio.events_disabled(),
            radio.tasks_txen(),
            Some(task_group_disable),
        );
        self.include_group(3);

        // (timer.comp1) -> (egu.trigger(timeout) -> SWI, group.dis)
        channels |= self.connect(
            4,
            timer.events_compare(1),
            egu.tasks_trigger(timeout),
            Some(task_group_disable),
        );

        // timer.comp1 -> egu.trigger(timeslotEnd)
        channels |= self.connect(
            5,
            timer.events_compare(1),
            egu.tasks_trigger(timeslot_end),
            None,
        );

        // (radio.disabled) -> (egu.trigger(timeslotEnd) -> SWI, timer.shutdown)
        channels |= self.connect(
            6,
            radio.events_disabled(),
            egu.tasks_trigger(timeslot_end),
            Some(timer.tasks_shutdown()),
        );
        self.include_group(6);

        // egu.triggered(timeout) -> radio.disable, timer.shutdown
        channels |= self.connect(
            7,
            egu.events_triggered(timeout),
            radio.tasks_disable(),
            Some(timer.tasks_shutdown()),
        );

        self.enable_channels(channels);
    }

    pub(crate) fn configure_peripheral_tx_ack(&mut self, _radio: &mut YipRadio, _egu: &mut YipEgu) {
        let radio = _radio.regs();
        let egu = _egu.regs();

        self.disable();

        let channels = self.connect(
            0,
            radio.events_disabled(),
            egu.tasks_trigger(YipTrigger::Disabled as usize),
            None,
        );

        self.enable_channels(channels);
    }

    fn enable_channels(&self, channels: u32) {
        pac::PPI.chenset().write_value(Chen(channels));
    }

    pub(crate) fn enable(&mut self) {
        pac::PPI.chenset().write_value(self.mask);
    }

    pub(crate) fn disable(&mut self) {
        pac::PPI.chenclr().write_value(self.mask);
    }
}

impl Drop for YipPpi<'_> {
    fn drop(&mut self) {
        let ppi = embassy_nrf::pac::PPI;

        self.disable();

        for channel in &self.channels {
            let number = channel.number();
            ppi.ch(number).eep().write_value(0);
            ppi.ch(number).tep().write_value(0);
            ppi.fork(number).tep().write_value(0);
        }

        ppi.chg(self.group.number()).write(|_| ());
    }
}
