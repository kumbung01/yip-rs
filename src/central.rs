use crate::hardware::api::{ClockDriver, EguDriver, PpiDriver, RadioDriver};
use core::cell::RefCell;
use embassy_nrf::interrupt::{self, InterruptExt};
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};

use crate::link::LinkSubstate::{self, Normal};
use crate::packet::YipSessionEndpoints;
use crate::packetbuffer::PacketBuffer;
use crate::payload::{ControlCommand, ControlHeader, DownHeader, SyncPacket, UpHeader};
use crate::pdu::PacketKind;
use crate::pdu::PacketType::*;

use crate::config::*;
use crate::hardware::device::DeviceContext;
use crate::hardware::egu::Trigger;
use crate::hardware::impl_yip_egu_handlers;
use crate::hop::get_channel;
use crate::link::{Link, LinkState::*};
use crate::packet::CurrentPacket;
use crate::packet::PacketChannels;
use crate::role::private::*;
use crate::session::YipSession;
use crate::signal::RssiState;

struct Alive(u32);

impl Alive {
    fn is_timeout(&mut self, slot_now: u32) -> bool {
        let limit_slots = TIMEOUT_LIMIT_US_CENTRAL.div_ceil(YIP_SLOTSIZE_US as u32);

        slot_now.wrapping_sub(self.0) >= limit_slots
    }

    fn update_alive(&mut self, slot_now: u32) {
        self.0 = slot_now;
    }
}

struct Slot {
    pub link: Link,
    pub rssi: RssiState,
    pub alive: Alive,
    pub txbuf: PacketBuffer,
    pub pkind: PacketKind,
}

impl Slot {
    fn reset(&mut self) {
        self.link.reset();
        self.rssi.reset();
        self.alive = Alive(0);
        self.txbuf.clear();
        self.pkind = PacketKind::Empty;
    }
}

pub fn init<const W: usize>(device: DeviceContext<YipInterruptHandler>) -> [YipSession; W] {
    #[cfg(feature = "defmt")]
    crate::hardware::cyccnt::init();

    let irq = device.egu.irq();

    IRQ_STACK.lock(|cell| {
        *cell.borrow_mut() = Some(YipCentral::new(device, W));
    });

    unsafe {
        // SAFETY: 생성자의 Binding으로 벡터를 검증했고,
        // stack 초기화와 ISR 등록이 완료됨.
        irq.enable();
    }

    let eps = YipCentral::session_endpoints();
    eps.map(|ep| YipSession::new(ep))
}

pub fn start() {
    IRQ_STACK.lock(|cell| {
        cell.borrow_mut()
            .as_mut()
            .expect("stack not initialized")
            .start();
    });
}

pub fn stop() {
    IRQ_STACK.lock(|cell| {
        cell.borrow_mut()
            .as_mut()
            .expect("stack not initialized")
            .stop();
    });
}

pub struct YipCentral {
    state: CentralState,
    turn: usize,
    slots: [Slot; YIP_MAX_LINK_COUNT],
    device: DeviceContext<YipInterruptHandler>,
}

#[repr(u8)]
#[derive(Clone, Copy, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum CentralState {
    Idle,
    Tx,
}

static CHANNELS: PacketChannels<YIP_MAX_LINK_COUNT> = PacketChannels::new();

pub(crate) static IRQ_STACK: Mutex<CriticalSectionRawMutex, RefCell<Option<YipCentral>>> =
    Mutex::new(RefCell::new(None));

pub struct YipInterruptHandler;

impl_yip_egu_handlers!();

impl RoleHeader for YipCentral {
    type TxHeader = DownHeader;
    type RxHeader = UpHeader;
}

impl YipCentral {
    pub(crate) fn start(&mut self) {
        self.setup();
        self.device.clock.start_central();
        self.prepare_tx();
    }

    pub(crate) fn stop(&mut self) {
        let DeviceContext {
            radio,
            clock,
            ppi,
            egu,
            ..
        } = &mut self.device;

        ppi.clear();
        clock.stop_central();
        radio.disable();
        egu.clear_all();

        for (i, slot) in self.slots.iter_mut().enumerate() {
            slot.reset();
            CHANNELS.link_ep(i).clear();
        }

        self.state = CentralState::Idle;
    }

    pub fn setup(&mut self) {
        let DeviceContext {
            clock,
            ppi,
            egu,
            context,
            ..
        } = &mut self.device;
        #[cfg(feature = "defmt")]
        defmt::debug!("central setup");

        context.slot_ticks = YIP_SLOTSIZE_US as u32;
        ppi.configure_central(clock, egu);
        self.state = CentralState::Idle;
    }

    fn on_interrupt(&mut self) {
        use CentralState::*;
        use Trigger::*;

        if let Some(trigger) = self.device.egu.get_and_clear_triggered() {
            // debug!("state {} trig {}", self.state, trigger);
            match (self.state, trigger) {
                (Idle, Start) => self.prepare_tx(),
                (Tx, TimeslotEnd) => self.timeslot_end(),
                _ => {}
            }
        }
    }
}

impl YipCentral {
    pub(crate) fn new(device: DeviceContext<YipInterruptHandler>, pipes: usize) -> Self {
        assert!(pipes <= YIP_MAX_LINK_COUNT);

        let mut central = Self {
            state: CentralState::Idle,
            turn: 0,
            device,
            slots: core::array::from_fn(|_| Slot {
                alive: Alive(0),
                link: Link::new(),
                rssi: RssiState::new(),
                txbuf: PacketBuffer::new(),
                pkind: PacketKind::Empty,
            }),
        };

        central.device.set_pipes(pipes);

        central
    }

    fn prepare_tx(&mut self) {
        let DeviceContext {
            radio,
            clock,
            ppi,
            egu,
            context: ctx,
            ..
        } = &mut self.device;

        ppi.disable();

        self.state = CentralState::Tx;

        let slot_diff = clock.set_next_slot_central();

        ctx.refslot = ctx.refslot.wrapping_add(slot_diff);

        self.turn = (self.turn + slot_diff as usize) % ctx.pipes;

        let Slot {
            link,
            rssi,
            txbuf: buf,
            pkind: ptype,
            ..
        } = &mut self.slots[self.turn];
        let ep = CHANNELS.link_ep(self.turn);

        buf.clear();
        let mut body = buf.write_header::<Self>(DownHeader { rssi: rssi.get() });

        if link.current_tx.is_none() {
            link.current_tx = ep.tx.try_receive().map(CurrentPacket::new);
        }

        let (packet_type, more_frag) = match link.state {
            Disconnected => {
                body.write(ControlHeader {
                    command: ControlCommand::Sync,
                });

                body.write(SyncPacket {
                    refslot: ctx.refslot,
                    slot_ticks: ctx.slot_ticks,
                    pipes: ctx.pipes as u8,
                });
                *ptype = PacketKind::Control;
                (Control, link.current_tx.is_some())
            }

            Connected(substate) => match substate {
                LinkSubstate::Normal => {
                    if let Some(packet) = link.current_tx.as_mut() {
                        let is_new = packet.is_new();
                        if is_new {
                            body.write_bytes(&[packet.total_len() as u8]);
                        }

                        let frag = packet.get_fragment(body.free_size());
                        body.write_bytes(frag);

                        *ptype = PacketKind::Data;

                        (
                            if is_new { Start } else { Continue },
                            packet.remain_after_tx() > 0,
                        )
                    } else {
                        body.write_bytes(&[0]);
                        *ptype = PacketKind::Empty;
                        (Start, false)
                    }
                }
            },
        };

        let pdu = &mut buf.pdu1;

        pdu.set_packet_type(packet_type);

        pdu.set_sn(link.sn);
        pdu.set_nesn(link.nesn);
        pdu.set_md(more_frag || ep.tx.queue.size() > 0);

        let channel = get_channel(ctx.refslot as u32);
        radio.set_channel(channel);

        unsafe {
            radio.set_packetptr(buf.as_mut_ptr());
        }

        let _timeout = clock.set_timeout_central(buf.len());

        radio.set_txaddress(self.turn as u8);
        radio.set_rxaddress(self.turn as u8);

        egu.select_interrupt(Trigger::TimeslotEnd);

        ppi.enable();
    }

    fn timeslot_end(&mut self) {
        let DeviceContext {
            radio,
            context: ctx,
            ppi,
            ..
        } = &mut self.device;

        ppi.disable();

        let Slot {
            link,
            rssi,
            alive,
            txbuf: buf,
            pkind,
        } = &mut self.slots[self.turn];
        let ep = CHANNELS.link_ep(self.turn);

        let events = radio.take_interrupt_events();

        if !events.crc_ok {
            if matches!(link.state, Connected(..)) {
                if link.current_tx.is_some() {
                    link.tx_attempts += 1;
                }

                if alive.is_timeout(ctx.refslot) {
                    link.state = Disconnected;
                    ep.conn.signal(Disconnected);
                    link.reset();
                    rssi.reset();
                    ep.clear();
                }
            }

            self.prepare_tx();
            return;
        }

        alive.update_alive(ctx.refslot);
        if link.state == Disconnected {
            link.state = Connected(Normal);
            ep.conn.signal(Connected(Normal));
        }

        let rssi_sample = radio.get_rssi_sample();
        if buf.len() >= RADIO_RSSI_PADDING_BYTES {
            rssi.update(rssi_sample);
        }

        // break if we have to retry
        'rx: {
            // Peer SN differs from the expected SN, so this is a duplicate.
            let duplicate_payload = buf.pdu1.sn() != link.nesn;

            if duplicate_payload {
                debug!("duplicate");
                break 'rx;
            }

            let rx_packet_type = buf.pdu1.packet_type();

            let Ok((_header, body)) = buf.read_header::<Self>() else {
                debug!("invalid rx header");

                self.prepare_tx();

                return;
            };

            match rx_packet_type {
                Control => 'rx_control: {
                    let Some((&byte, _data)) = body.as_slice().split_first() else {
                        break 'rx_control;
                    };

                    let Ok(command) = ControlCommand::try_from(byte) else {
                        break 'rx_control;
                    };

                    match command {
                        ControlCommand::Sync => {
                            // noop, for now
                            error!("peripheral should not send this");
                        }
                    }
                }

                Start => 'rx_start: {
                    let Some((&total_len, data)) = body.as_slice().split_first() else {
                        debug!("missing packet length, drop");
                        link.current_rx = None;
                        break 'rx_start;
                    };

                    if total_len == 0 {
                        debug!("empty packet, skip");
                        link.current_rx = None;
                        break 'rx;
                    }

                    let rx_packet = link
                        .current_rx
                        .take()
                        .or_else(|| ep.rx.try_acquire().map(CurrentPacket::new));

                    let Some(mut rx_packet) = rx_packet else {
                        debug!("buf get fail, retry");
                        break 'rx;
                    };

                    rx_packet.clear();

                    if rx_packet.set_total_len(total_len as usize).is_err() {
                        break 'rx_start;
                    }

                    if rx_packet.append_from_slice(data).is_err() {
                        break 'rx_start;
                    }

                    if rx_packet.remaining_len() == 0 {
                        ep.rx.send(rx_packet.packet());
                    } else {
                        link.current_rx = Some(rx_packet);
                    }
                }

                Continue => 'rx_cont: {
                    let data = body.as_slice();

                    if data.is_empty() {
                        debug!("{} data must not be empty!", Continue);
                        link.current_rx = None;

                        break 'rx_cont;
                    }

                    let Some(mut rx_packet) = link.current_rx.take() else {
                        debug!("we must have packet here! skip this packet");
                        break 'rx_cont;
                    };

                    if rx_packet.append_from_slice(data).is_err() {
                        debug!("drop this packet");
                        break 'rx_cont;
                    }

                    if rx_packet.remaining_len() == 0 {
                        ep.rx.send(rx_packet.packet());
                    } else {
                        link.current_rx = Some(rx_packet);
                    }
                }

                Invalid => {
                    debug!("data has been corrupted, skip this packet");
                }
            };

            link.nesn = !link.nesn;

            debug!("rx type {} len {}", rx_packet_type, buf.len());
            debug!("rx data: {}", buf.data()[0..buf.len()]);
        }

        'tx_commit: {
            if *pkind == PacketKind::Empty {
                break 'tx_commit;
            }

            // peripheral requested same sn, so tx failed
            let tx_failed = buf.pdu1.nesn() == link.sn;

            if tx_failed {
                link.tx_attempts += 1;
                break 'tx_commit;
            }

            link.sn = !link.sn;
            link.tx_attempts = 0;

            if *pkind == PacketKind::Data {
                let tx_packet = link
                    .current_tx
                    .as_mut()
                    .expect("packet must exists when Data");

                if tx_packet.commit_fragment() == 0 {
                    link.current_tx = None;
                }
            }
        }

        self.prepare_tx();
    }

    pub fn session_endpoints<const W: usize>() -> [YipSessionEndpoints; W] {
        assert!(W <= YIP_MAX_LINK_COUNT);

        core::array::from_fn(|i| CHANNELS.session_ep(i))
    }
}
