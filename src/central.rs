use crate::header::{DownHeader, UpHeader, YipRoleHeader};
use crate::packetbuffer::{YipControlPacket, YipPacketBuffer};

use crate::config::*;
use crate::device::YipDevice;
use crate::egu::YipTrigger;
use crate::hop::get_channel;
use crate::link::{LinkState, RssiState, YipLink};
use crate::packet::CurrentPacket;
use crate::role::{YipRole, YipRoleStack};

pub struct YipCentral {
    state: YipCentralState,
    turn: usize,
}

#[repr(u8)]
#[derive(Clone, Copy, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum YipCentralState {
    Idle,
    Tx,
}

impl YipRole for YipCentral {}

impl YipRoleHeader for YipCentral {
    type TxHeader = DownHeader;
    type RxHeader = UpHeader;
}

impl<'q, 'p, const W: usize> YipRoleStack<'q, 'p, W> for YipCentral {
    fn new() -> Self {
        Self {
            state: YipCentralState::Idle,
            turn: 0,
        }
    }

    fn setup(&mut self, device: &mut YipDevice, _: u8) {
        let YipDevice {
            radio,
            timer,
            ppi,
            egu,
            context,
            ..
        } = device;

        defmt::debug!("central setup");

        context.set_slotsize();
        ppi.configure_central(radio, timer, egu);
        timer.start();
    }

    fn on_interrupt(
        &mut self,
        device: &mut YipDevice,
        links: &mut [YipLink<'q, 'p, W>],
        trigger: YipTrigger,
        buffer: &mut YipPacketBuffer<Self>,
        _unused_buf: &mut YipPacketBuffer<Self>,
    ) {
        use YipCentralState::*;
        use YipTrigger::*;

        defmt::debug!("state: {}, trig: {}", self.state, trigger);

        match (self.state, trigger) {
            (Idle, Start) => self.prepare_tx(device, links, buffer),
            (Tx, TimeslotEnd) => self.timeslot_end(device, links, buffer),
            _ => {}
        }
    }
}

impl YipCentral {
    fn prepare_tx<'q, 'p, const LINK_COUNT: usize>(
        &mut self,
        device: &mut YipDevice,
        links: &mut [YipLink<'q, 'p, LINK_COUNT>],
        buffer: &mut YipPacketBuffer<Self>,
    ) {
        let YipDevice {
            radio,
            timer,
            ppi,
            egu,
            context,
            ..
        } = device;

        ppi.disable();

        self.state = YipCentralState::Tx;

        let timer_now = timer.cc(3).capture();
        let timer_elapsed = timer_now.wrapping_sub(context.start);
        let radio_margin_timer_ticks = us_to_timer_ticks(RADIO_MARGIN_US) as u32;
        let slot_diff = (timer_elapsed + radio_margin_timer_ticks).div_ceil(SLOTSIZE_TIMER_TICKS);
        let next_slot = context.refslot.wrapping_add(slot_diff);
        let tx_start = context
            .start
            .wrapping_add(slot_diff.wrapping_mul(SLOTSIZE_TIMER_TICKS));

        context.refslot = next_slot;
        context.start = tx_start;

        let pipes = links.len();

        self.turn = (self.turn + slot_diff as usize) % pipes;

        let link = &mut links[self.turn];

        buffer.clear();

        {
            let mut body = buffer
                .write_header(DownHeader {
                    rssi: link.rssi.get(),
                })
                .expect("invalid header size");

            if link.state == LinkState::Disconnected {
                let control_packet = YipControlPacket {
                    refslot: context.refslot,
                    slotsize_ticks: context.slotsize_ticks,
                    pipes: pipes as u8,
                };
                body.write(control_packet)
                    .expect("fragment must fit in TX buffer");
                buffer.pdu1.set_more(false);
            } else {
                if link.current_tx.is_none() {
                    link.current_tx = link.endpoints.tx.try_receive().map(CurrentPacket::new);
                }

                if let Some(packet) = &mut link.current_tx {
                    let available = body.free_size();
                    let fragment = packet.get_fragment(available);

                    body.write(fragment)
                        .expect("fragment must fit in TX buffer");
                    buffer.pdu1.set_more(packet.is_more());
                }
            }
        }

        buffer.pdu1.set_ctrl(link.state == LinkState::Disconnected);
        buffer.pdu1.set_sn(link.sn);
        buffer.pdu1.set_nesn(link.nesn);

        radio.set_txaddress(self.turn as u8);
        radio.set_rxaddress(self.turn as u8);

        let channel = get_channel(context.refslot as u32);
        radio.set_channel(channel);
        radio.set_whiten_seed(channel);

        unsafe {
            radio.set_packetptr(buffer.as_mut_ptr());
        }

        timer.cc(0).write(tx_start);

        let timeout = tx_start.wrapping_add(context.calculate_timeout_timer(buffer.len()));
        timer.cc(1).write(timeout);

        egu.select_interrupt(YipTrigger::TimeslotEnd);

        ppi.enable();

        let timer_end = timer.cc(3).capture();

        defmt::debug!(
            "prepare_tx: now: {} start: {} timeout {} codetime {} us slot {:x} turn {} ch {} ",
            timer_now,
            tx_start,
            timeout,
            timer_ticks_to_us(timer_end.wrapping_sub(timer_now)),
            next_slot,
            self.turn,
            channel
        );
        defmt::debug!(
            "len {} ctrl {} sn {} nesn {} more {}",
            buffer.len(),
            buffer.pdu1.ctrl(),
            buffer.pdu1.sn(),
            buffer.pdu1.nesn(),
            buffer.pdu1.more(),
        );
    }

    fn timeslot_end<'q, 'p, const LINK_COUNT: usize>(
        &mut self,
        device: &mut YipDevice,
        links: &mut [YipLink<'q, 'p, LINK_COUNT>],
        buffer: &mut YipPacketBuffer<Self>,
    ) {
        let YipDevice {
            radio,
            timer,
            context,
            ppi,
            ..
        } = device;

        ppi.disable();

        let link = &mut links[self.turn];

        let events = radio.take_interrupt_events();
        if !events.crc_ok {
            if link.state == LinkState::Connected {
                if link.current_tx.is_some() {
                    link.tx_attempts += 1;
                }

                if link.check_timeout_central(context.start) == LinkState::Disconnected {
                    link.reset();
                    link.endpoints.conn.signal(LinkState::Disconnected);
                }
            }

            let timer_now = timer.cc(3).capture();
            defmt::debug!(
                "slot timeout now {}, elapsed {} us",
                timer_now,
                timer_elapsed_us(timer_now, context.start)
            );

            let tx_end = timer.cc(2).read();

            defmt::debug!(
                "TX end: {}, duration: {} us",
                tx_end,
                timer_elapsed_us(tx_end, context.start)
            );

            self.prepare_tx(device, links, buffer);
            return;
        }

        let my_nesn = link.nesn;
        let my_sn = link.sn;
        let my_crc = link.last_rx_crc;

        let your_nesn = buffer.pdu1.nesn();
        let your_sn = buffer.pdu1.sn();
        let your_crc = radio.get_rxcrc();

        let is_ctrl = buffer.pdu1.ctrl();
        let is_more = buffer.pdu1.more();

        let rssi_sample = radio.get_rssi_sample();

        // peripheral requested next sn, so tx succeded
        if your_nesn != my_sn {
            link.sn = !link.sn;
            if let Some(packet) = &mut link.current_tx {
                if packet.commit_fragment() == 0 {
                    link.current_tx = None;
                    // set event?
                }
            }
        }

        link.update_link_state(context.start);

        if buffer.len() >= 4 {
            link.rssi.update(rssi_sample);
        }

        let retransmit_payload = (your_crc == my_crc) && (your_sn == my_nesn);

        'rx: {
            if retransmit_payload {
                break 'rx;
            }

            let Ok((_, body)) = buffer.read_header() else {
                break 'rx;
            };

            if !is_ctrl {
                if link.current_rx.is_none() {
                    let Some(handle) = link.endpoints.rx.try_acquire() else {
                        break 'rx;
                    };
                    link.current_rx = Some(CurrentPacket::new(handle));
                }

                let Some(rx_packet) = &mut link.current_rx else {
                    break 'rx;
                };

                if rx_packet.append_from_slice(body.as_slice()).is_err() {
                    break 'rx;
                }

                if !is_more {
                    if let Some(rx_packet) = link.current_rx.take() {
                        link.endpoints.rx.send(rx_packet.packet());
                    }
                }
            }

            link.last_rx_crc = your_crc;
            link.nesn = !link.nesn;
        }

        self.prepare_tx(device, links, buffer);
    }
}
