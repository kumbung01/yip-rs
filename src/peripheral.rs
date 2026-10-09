use crate::config::*;
use crate::device::YipDevice;
use crate::egu::YipTrigger;
use crate::header::{DownHeader, UpHeader, YipRoleHeader};
use crate::hop::get_channel;
use crate::link::YipLink;
use crate::packetbuffer::{YipControlPacket, YipPacketBuffer};
use crate::radio::RadioEvents;
use crate::role::{YipRole, YipRoleStack};
use crate::timing::ScheduledSlot;

#[derive(Default)]
pub(crate) struct PeripheralContext {
    last_hb: u32,
    drift: i32,
    drift_now: i32,
    hb_loops: u32,
    windowsize: u32,
    windowsize_us: u32,
    slotdiff_now: u32,
    timeout_cnt: usize,

    is_hb: bool,
    hb_failed: bool,
    more_data: bool,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum YipPeripheralState {
    Idle,
    Syncing,
    RxReady,
    Rx,
    TxAck,
}

pub struct YipPeripheral {
    timing: PeripheralContext,
    state: YipPeripheralState,
    id: u8,
}

impl YipRole for YipPeripheral {}

impl YipRoleHeader for YipPeripheral {
    type TxHeader = UpHeader;
    type RxHeader = DownHeader;
}

impl<'q, 'p> YipRoleStack<'q, 'p, 1> for YipPeripheral {
    fn new() -> Self {
        Self {
            timing: PeripheralContext::default(),
            state: YipPeripheralState::Idle,
            id: 0,
        }
    }

    fn setup(&mut self, device: &mut YipDevice, id: u8) {
        let YipDevice {
            radio,
            rtc,
            ppi,
            egu,
            context,
            ..
        } = device;

        defmt::debug!("peripheral setup");
        self.id = id;
        radio.set_rxaddress(id);
        rtc.enable();
    }

    fn on_interrupt(
        &mut self,
        device: &mut YipDevice,
        links: &mut [YipLink<'q, 'p, 1>],
        trigger: YipTrigger,
        rx_buffer: &mut YipPacketBuffer<Self>,
        tx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        device.ppi.disable();
        self.state_machine(device, &mut links[0], trigger, rx_buffer, tx_buffer);
    }
}

impl<'q, 'p> YipPeripheral {
    fn state_machine(
        &mut self,
        device: &mut YipDevice,
        link: &mut YipLink<'q, 'p, 1>,
        trigger: YipTrigger,
        rx_buffer: &mut YipPacketBuffer<Self>,
        tx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        use YipPeripheralState::*;
        use YipTrigger::*;

        match (self.state, trigger) {
            (Idle, Start) => self.sync_start(device, link, rx_buffer),
            (Syncing, TimeslotEnd) => self.sync_disabled(device, link, rx_buffer),
            (RxReady, Start) => self.prepare_next_rx(device, link, rx_buffer),
            (Rx, TimeslotEnd) => self.disabled_rx(device, link, rx_buffer, tx_buffer),
            (TxAck, Disabled) => self.disabled_tx_ack(device, link, rx_buffer),
            _ => {}
        }
    }

    fn sync_start(
        &mut self,
        device: &mut YipDevice,
        link: &mut YipLink<'q, 'p, 1>,
        rx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        let YipDevice {
            radio,
            timer,
            rtc,
            ppi,
            egu,
            context,
        } = device;

        self.state = YipPeripheralState::Syncing;

        let rtc_now = rtc.read();
        let slot = context.clock.next_slot(rtc_now, 1000000);

        let ScheduledSlot {
            slot_diff,
            timer_cc,
            rtc_cc,
        } = slot;

        rtc.set_compare(RTC_CC0, rtc_cc).expect("invalid value");
        timer.cc(0).write(timer_cc);

        let timeout = timer_cc + 1000_000 - 500;
        timer.cc(1).write(timeout);

        context.refslot += slot_diff;

        let channel = get_channel(context.refslot as u32);
        radio.set_channel(channel);
        radio.set_whiten_seed(channel);

        unsafe {
            radio.set_packetptr(rx_buffer.as_mut_ptr());
        }

        ppi.configure_sync(radio, timer, rtc, egu);
        egu.select_interrupt(YipTrigger::TimeslotEnd);
    }

    fn sync_disabled(
        &mut self,
        device: &mut YipDevice,
        link: &mut YipLink<'q, 'p, 1>,
        rx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        let events = device.radio.take_interrupt_events();

        if !events.crc_ok || !rx_buffer.pdu1.ctrl() {
            self.sync_start(device, link, rx_buffer);
            return;
        }

        let body = rx_buffer
            .read_header()
            .and_then(|(_, reader)| YipControlPacket::try_from(reader));

        let Ok(body) = body else {
            self.sync_start(device, link, rx_buffer);
            return;
        };

        let context = &mut device.context;
        context.refslot = body.refslot;
        context.slotsize_ticks = body.slotsize_ticks;
        context.pipes = body.pipes;
        context.clock.set(device.timer.cc(2).read());

        link.sn = rx_buffer.pdu1.sn();
        link.nesn = rx_buffer.pdu1.nesn();

        self.prepare_rx(device, link, rx_buffer);
    }

    fn prepare_rx(
        &mut self,
        device: &mut YipDevice,
        link: &mut YipLink<'q, 'p, 1>,
        rx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        let YipDevice {
            radio,
            timer,
            rtc,
            ppi,
            egu,
            context,
        } = device;

        self.state = YipPeripheralState::RxReady;

        // set next rtc wakeup here
        let rtc_now = rtc.read();
        let slot = context.clock.next_slot(rtc_now, context.slotsize_ticks);
        let ScheduledSlot {
            slot_diff,
            rtc_cc,
            timer_cc,
        } = slot;

        ppi.configure_peripheral_prepare_rx(rtc, egu);
        egu.select_interrupt(YipTrigger::Start);
    }

    fn prepare_next_rx(
        &mut self,
        device: &mut YipDevice,
        link: &mut YipLink<'q, 'p, 1>,
        rx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        let YipDevice {
            radio,
            timer,
            rtc,
            ppi,
            egu,
            context,
        } = device;

        self.state = YipPeripheralState::Rx;

        // set next timeslot cc here

        egu.select_interrupt(YipTrigger::TimeslotEnd);
        ppi.configure_peripheral_next_rx(radio, timer, rtc, egu);
    }

    fn disabled_rx(
        &mut self,
        device: &mut YipDevice,
        link: &mut YipLink<'q, 'p, 1>,
        rx_buffer: &mut YipPacketBuffer<Self>,
        tx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        let YipDevice {
            radio,
            timer,
            rtc,
            ppi,
            egu,
            context,
        } = device;

        let events = radio.take_interrupt_events();
        if !events.crc_ok {
            // do some timeout handle here
            return;
        }

        // do some tx ack stuff here
        //
        // after tx, we set ppi
        egu.select_interrupt(YipTrigger::Disabled);
        ppi.configure_peripheral_tx_ack(radio, egu);
        self.state = YipPeripheralState::TxAck;

        // do some rx handle stuff here
    }

    fn disabled_tx_ack(
        &mut self,
        device: &mut YipDevice,
        link: &mut YipLink<'q, 'p, 1>,
        rx_buffer: &mut YipPacketBuffer<Self>,
    ) {
        device.ppi.disable();

        self.prepare_rx(device, link, rx_buffer);
    }
}
