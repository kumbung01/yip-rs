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
// READY fallback uses an EGU event whose interrupt is never enabled.
const ACK_START: usize = 3;
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
    completion_group: Option<PpiGroup<'static>>,
    completion_mask: u32,
}

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
            completion_group: None,
            completion_mask: 0,
        };
        this.clear_all();
        this
    }

    pub(crate) fn set_central_completion_group(&mut self, group: PpiGroup<'static>) {
        pac::PPI.chg(group.number()).write(|_| ());
        self.completion_group = Some(group);
    }

    fn include_group(&mut self, channel: usize) {
        pac::PPI.chg(self.group.number()).modify(|w| {
            w.set_ch(self.channels[channel].number(), true);
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

    /// One startup measurement, before any radio routes are installed.
    pub(crate) fn measure_rtc_ratio(&mut self, clock: &mut Nrf52Clock) {
        use embassy_nrf::{
            rtc::{CompareChannel, Interrupt},
            timer::Frequency,
        };

        self.clear_all();
        clock.rtc.disable();
        clock.timer.stop();
        clock.timer.clear();
        clock.timer.set_frequency(Frequency::F16MHz);
        let rtc = clock.rtc.regs();
        let timer = clock.timer.regs();
        let start = crate::config::rtc_wrap(clock.rtc.read().wrapping_add(8));
        let end = crate::config::rtc_wrap(start.wrapping_add(1024));
        clock
            .rtc
            .set_compare(CompareChannel::_1, start)
            .expect("valid RTC compare");
        clock
            .rtc
            .set_compare(CompareChannel::_2, end)
            .expect("valid RTC compare");
        rtc.events_compare(1).write_value(0);
        rtc.events_compare(2).write_value(0);
        timer.cc(2).write_value(0);
        timer.cc(3).write_value(0);
        clock.rtc.enable_event(Interrupt::Compare1);
        clock.rtc.enable_event(Interrupt::Compare2);
        let channels = self.connect(0, rtc.events_compare(1), timer.tasks_capture(2), None)
            | self.connect(1, rtc.events_compare(2), timer.tasks_capture(3), None);
        self.enable_channels(channels);
        clock.timer.start();
        clock.rtc.enable();

        // Captures are hardware driven. A one-second HF deadline also detects a missing LF clock.
        let measured = loop {
            let last = timer.cc(3).read();
            if last != 0 {
                break Some(last.wrapping_sub(timer.cc(2).read()));
            }
            timer.tasks_capture(0).write_value(1);
            if timer.cc(0).read() >= 16_000_000 {
                break None;
            }
            core::hint::spin_loop();
        };

        self.clear_all();
        clock.rtc.disable_event(Interrupt::Compare1);
        clock.rtc.disable_event(Interrupt::Compare2);
        clock.rtc.disable();
        rtc.events_compare(1).write_value(0);
        rtc.events_compare(2).write_value(0);
        rtc.tasks_clear().write_value(1);
        clock.timer.stop();
        clock.timer.clear();
        clock.timer.set_frequency(crate::config::TIMER_FREQUENCY);
        clock.ratio =
            super::rtc_ratio::RtcRatio::measured(measured.expect("RTC/HF calibration timed out"))
                .expect("RTC/HF calibration outside supported clock range");
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
        if let Some(group) = &self.completion_group {
            ppi.chg(group.number()).write(|_| ());
        }
        self.completion_mask = 0;
    }

    fn enable(&mut self) {
        // Completion stays disarmed until the scheduled TX actually starts.
        pac::PPI
            .chenset()
            .write_value(Chen(self.mask.0 & !self.completion_mask));
    }

    fn disable(&mut self) {
        pac::PPI.chenclr().write_value(self.mask);
    }

    fn configure_central(&mut self, clock: &mut Nrf52Clock, _egu: &mut Nrf52Egu) {
        self.clear_all();
        let ppi = pac::PPI;
        let radio = pac::RADIO;
        let timer = clock.timer.regs();
        let egu = _egu.regs();

        let timeslot_end = Trigger::TimeslotEnd.number();

        let group_num = self.group.number();
        let task_group_enable = ppi.tasks_chg(group_num).en();
        let task_group_disable = ppi.tasks_chg(group_num).dis();
        let completion_num = self
            .completion_group
            .as_ref()
            .expect("central requires a completion PPI group")
            .number();
        let completion_enable = ppi.tasks_chg(completion_num).en();
        let completion_disable = ppi.tasks_chg(completion_num).dis();
        self.completion_mask =
            (1u32 << self.channels[2].number()) | (1u32 << self.channels[3].number());
        ppi.chg(completion_num).write(|w| {
            w.set_ch(self.channels[2].number(), true);
            w.set_ch(self.channels[3].number(), true);
        });

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

        // First completion disarms both CRCOK and timeout before ISR dispatch.
        self.connect(
            2,
            timer.events_compare(1),
            egu.tasks_trigger(timeslot_end),
            Some(completion_disable),
        );

        self.connect(
            3,
            radio.events_crcok(),
            egu.tasks_trigger(timeslot_end),
            Some(completion_disable),
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

        // A PPI channel has one fork; use a spare channel for the second group.
        self.connect(6, timer.events_compare(0), completion_enable, None);
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
            egu.tasks_trigger(Trigger::TimeslotEnd.number()),
            Some(timer.tasks_shutdown()),
        );

        self.enable_channels(channels);
    }

    fn configure_rx(&mut self, _clock: &mut Nrf52Clock, _egu: &mut Nrf52Egu) {
        // nRF52 still installs its routes per phase. Leave sync disarmed
        // before the role schedules its first connected receive window.
        self.clear();
    }

    fn configure_peripheral_prepare_rx(&mut self, _clock: &mut Nrf52Clock, _egu: &mut Nrf52Egu) {
        let rtc = _clock.rtc.regs();
        let egu = _egu.regs();

        self.clear();

        let channels = self.connect(
            0,
            rtc.events_compare(0),
            egu.tasks_trigger(Trigger::Start.number()),
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

        let timeslot_end = Trigger::TimeslotEnd.number();

        let group_num = self.group.number();
        let task_group_enable = ppi.tasks_chg(group_num).en();
        let task_group_disable = ppi.tasks_chg(group_num).dis();

        self.clear();

        // RX starts automatically; ACK TX waits in TXIDLE until its buffer
        // and completion route have been installed.
        radio.shorts().modify(|w| {
            w.set_ready_start(false);
            w.set_txready_start(false);
            w.set_rxready_start(true);
        });

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
        self.include_group(3);
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

        let completion = self.connect(
            0,
            radio.events_disabled(),
            egu.tasks_trigger(Trigger::TimeslotEnd.number()),
            None,
        );

        self.enable_channels(completion);

        egu.events_triggered(ACK_START).write_value(0);
        let group_disable = pac::PPI.tasks_chg(self.group.number()).dis();
        self.include_group(1);
        self.include_group(2);
        let mut start = self.connect(
            1,
            radio.events_txready(),
            radio.tasks_start(),
            Some(group_disable),
        );
        start |= self.connect(
            2,
            egu.events_triggered(ACK_START),
            radio.tasks_start(),
            Some(group_disable),
        );

        // Make ACK buffer/register writes visible before permitting DMA START.
        cortex_m::asm::dsb();
        self.enable_channels(start);
        // READY may have occurred before the PPI route was enabled. Use the
        // same one-shot group so hardware and fallback cannot start twice.
        if radio.state().read().state() == pac::radio::vals::State::TxIdle {
            egu.tasks_trigger(ACK_START).write_value(1);
        }
    }
}
