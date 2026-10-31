#[cfg(feature = "defmt")]
use crate::hardware::api::ClockTimestamp;
use crate::hardware::api::{ClockDriver, EguDriver, PpiDriver, RadioDriver};
use core::cell::RefCell;
use embassy_nrf::interrupt::{self, InterruptExt};
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};

use crate::hardware::Clock;
use crate::hardware::clock::Timing;
use crate::hardware::device::DeviceContext;
use crate::hardware::egu::Trigger::{Start, TimeslotEnd};
use crate::hardware::impl_yip_egu_handlers;
use crate::session::YipSession;
use crate::{config::*, enable_start};

use crate::hop::get_channel;
use crate::link::{Link, LinkState::*, LinkSubstate::*};

use crate::packet::*;
use crate::packetbuffer::*;
use crate::payload::{ControlCommand, DownHeader, Payload, SyncPacket, UpHeader};
use crate::pdu::PacketKind;
use crate::pdu::PacketType;
use crate::role::private::*;
use crate::signal::TxpowerState;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum PeripheralState {
    Idle,
    Syncing,
    RxReady,
    Rx,
    TxAck,
}

#[derive(Default)]
struct Retry {
    retry: Option<u32>,
    limit_slots: Option<u32>,
}

impl Retry {
    fn is_timeout(&mut self, slot_now: u32) -> bool {
        let limit_slots = self.limit_slots.expect("must exists here");
        let last_retry = self.retry.get_or_insert(slot_now);

        if slot_now.wrapping_sub(*last_retry) >= limit_slots {
            true
        } else {
            false
        }
    }

    fn set_limit(&mut self, slot_ticks: u32) {
        assert!(slot_ticks > 0, "slotsize must be greater than 0");
        self.limit_slots =
            Some((us_to_timer_ticks(TIMEOUT_LIMIT_US_PERIPHERAL) as u32).div_ceil(slot_ticks));
    }

    fn clear(&mut self) {
        self.retry = None;
    }
}

struct Slot {
    link: Link,
    tx_power: TxpowerState,
    last_retry: Retry,
    txbuf: PacketBuffer,
    pkind: PacketKind,
}

impl Slot {
    fn reset(&mut self) {
        self.link.reset();
        self.tx_power.reset();
        self.last_retry.clear();
        self.txbuf.clear();
    }
}

pub struct YipPeripheral {
    timing: Timing,
    state: PeripheralState,
    slot: Slot,
    rxbuf: PacketBuffer,
    device: DeviceContext<YipInterruptHandler>,
    id: u8,
}

static CHANNELS: PacketChannels<1> = PacketChannels::new();

pub(crate) static IRQ_STACK: Mutex<CriticalSectionRawMutex, RefCell<Option<YipPeripheral>>> =
    Mutex::new(RefCell::new(None));

pub struct YipInterruptHandler;

impl_yip_egu_handlers!();

impl RoleHeader for YipPeripheral {
    type TxHeader = UpHeader;
    type RxHeader = DownHeader;
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

pub fn init(device: DeviceContext<YipInterruptHandler>, id: u8) -> YipSession {
    #[cfg(feature = "defmt")]
    crate::hardware::ppi_timing::init();

    let irq = device.egu.irq();

    let mut role = YipPeripheral::new(device, id);
    role.setup();

    IRQ_STACK.lock(|cell| {
        *cell.borrow_mut() = Some(role);
    });

    enable_start();

    unsafe {
        // SAFETY: 생성자의 Binding으로 벡터를 검증했고,
        // stack 초기화와 ISR 등록이 완료됨.
        irq.enable();
    }

    let [ep] = YipPeripheral::session_endpoints();
    YipSession::new(ep)
}

impl YipPeripheral {
    pub(crate) fn start(&mut self) {
        self.setup();
        self.device.clock.start_peripheral();
        self.sync_start();
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
        clock.stop_peripheral();
        radio.disable();
        egu.clear_all();

        self.slot.reset();
        CHANNELS.link_ep(0).clear();

        #[cfg(all(feature = "nrf54lm20", feature = "reduce-rram-latency"))]
        {
            embassy_nrf::pac::RRAMC
                .power()
                .lowpowerconfig()
                .modify(|w| {
                    w.set_mode(embassy_nrf::pac::rramc::vals::Mode::PowerOff);
                });
        }

        self.state = PeripheralState::Idle;
    }

    pub(crate) fn setup(&mut self) {
        let DeviceContext { radio, .. } = &mut self.device;

        radio.set_rxaddress(self.id);
        radio.set_txaddress(self.id);

        self.state = PeripheralState::Idle;
    }

    fn on_interrupt(&mut self) {
        if let Some(trigger) = self.device.egu.get_and_clear_triggered() {
            use PeripheralState::*;
            match (self.state, trigger) {
                (Idle, Start) => self.sync_start(),
                (Syncing, TimeslotEnd) => self.sync_disabled(),
                (RxReady, Start) => self.prepare_next_rx(),
                (Rx, TimeslotEnd) => self.disabled_rx(),
                (TxAck, TimeslotEnd) => self.disabled_tx_ack(),
                _ => {}
            }
        }
    }
}

impl YipPeripheral {
    pub(crate) fn new(device: DeviceContext<YipInterruptHandler>, id: u8) -> Self {
        Self {
            timing: Timing::default(),
            state: PeripheralState::Idle,
            id: id,
            device,
            slot: Slot {
                link: Link::new(),
                tx_power: TxpowerState::new(),
                last_retry: Retry::default(),
                txbuf: PacketBuffer::new(),
                pkind: PacketKind::Empty,
            },
            rxbuf: PacketBuffer::new(),
        }
    }

    fn resync(&mut self) {
        debug!("resync");
        self.device.ppi.clear();
        self.device.radio.disable();
        self.slot.reset();
        CHANNELS.link_ep(0).clear();
        self.sync_start();
    }

    fn sync_start(&mut self) {
        let DeviceContext {
            radio,
            clock,
            ppi,
            egu,
            context,
            ..
        } = &mut self.device;

        self.state = PeripheralState::Syncing;

        let (slot_diff, _timeout) = clock.set_next_sync();

        context.refslot += slot_diff;

        let channel = get_channel(context.refslot as u32);
        radio.set_channel(channel);

        unsafe {
            radio.set_packetptr(self.rxbuf.as_mut_ptr());
        }

        ppi.configure_sync(clock, egu);
        egu.select_interrupt(TimeslotEnd);

        #[cfg(feature = "defmt")]
        {
            let ts = clock.get_start_ts().unscaled();
            let cc = clock.get_start_hw();
            debug!(
                "start ts {} cc {} timeout {} slot {} ch {}",
                ts, cc, _timeout, context.refslot, channel
            );
        }
    }

    fn sync_disabled(&mut self) {
        let DeviceContext {
            radio,
            clock,
            ppi,
            egu,
            context: ctx,
            ..
        } = &mut self.device;

        let tm = &mut self.timing;
        let rxbuf = &mut self.rxbuf;

        let events = radio.take_interrupt_events();

        debug!(
            "sync disabled: now {}, rxready {}",
            clock.get_now_peripheral(),
            events.rxready
        );

        let sync_packet = 'parse: {
            if !events.crc_ok || rxbuf.pdu1.packet_type() != PacketType::Control {
                debug!(
                    "crcok {}, ptype {}",
                    events.crc_ok,
                    rxbuf.pdu1.packet_type()
                );
                break 'parse None;
            }

            let Ok((_, body)) = rxbuf.read_header::<Self>() else {
                debug!("header parse fail");
                break 'parse None;
            };

            let Some((&cmd, cmd_body)) = body.as_slice().split_first() else {
                break 'parse None;
            };

            if ControlCommand::try_from(cmd) != Ok(ControlCommand::Sync) {
                break 'parse None;
            }

            SyncPacket::decode(cmd_body).ok()
        };

        let Some(sync_packet) = sync_packet else {
            debug!("invalid sync packet");
            self.sync_start();
            return;
        };

        debug!("sync success");

        // set context here
        ctx.slot_ticks = sync_packet.slot_ticks;
        ctx.pipes = sync_packet.pipes as usize;

        // The sync packet carries the slot duration in microseconds.
        let Some(loop_us) = ctx
            .slot_ticks
            .checked_mul(ctx.pipes as u32)
            .filter(|&period| period > 0 && period <= HEARTBEAT_DURATION_US)
        else {
            error!("invalid connection loop duration");
            self.sync_start();
            return;
        };
        tm.loop_us = loop_us;
        tm.hb_loops = HEARTBEAT_DURATION_US / loop_us;
        let sync = clock
            .read_sync()
            .checked_sub(ADDR_DELAY_TICKS)
            .expect("ADDRESS capture must include address delay");

        let Ok(new_hb) = Clock::update_timestamp_by_sync(clock.get_start_ts(), sync) else {
            error!("hb set failed");
            self.sync_start();
            return;
        };

        tm.hb = new_hb;
        tm.drift_reset();

        ctx.refslot = sync_packet.refslot;

        self.slot.link.sn = rxbuf.pdu1.sn();
        self.slot.link.nesn = rxbuf.pdu1.nesn();
        self.slot.last_retry.set_limit(ctx.slot_ticks);
        self.slot.pkind = PacketKind::Empty;

        debug!(
            "slotsize {} pipes {} hb_loops {} refslot {} hb {}",
            ctx.slot_ticks, ctx.pipes, tm.hb_loops, ctx.refslot, tm.hb
        );

        // Explicitly switch from scan routes to connected RX/ACK routes.
        ppi.configure_rx(clock, egu);

        let ep = CHANNELS.link_ep(0);

        self.slot.link.state = Connected(Normal);
        ep.conn.signal(self.slot.link.state);

        // self.prepare_rx();
        self.prepare_next_rx();
    }

    fn prepare_rx(&mut self) {
        let DeviceContext {
            clock, ppi, egu, ..
        } = &mut self.device;

        self.state = PeripheralState::RxReady;

        clock.set_next_pre_slot(&self.timing);

        ppi.configure_peripheral_prepare_rx(clock, egu);
        egu.select_interrupt(Start);
    }

    fn prepare_next_rx(&mut self) {
        let DeviceContext {
            radio,
            clock,
            ppi,
            egu,
            context: ctx,
            ..
        } = &mut self.device;

        self.state = PeripheralState::Rx;

        let Ok((loop_diff, _timeout)) = clock.set_next_slot_peripheral(&self.timing) else {
            debug!("cannot get next slot, return to sync");
            self.sync_start();
            return;
        };

        self.timing.slot_now = ctx.refslot.wrapping_add(loop_diff * ctx.pipes as u32);
        let channel = get_channel(self.timing.slot_now);
        radio.set_channel(channel);

        unsafe {
            radio.set_packetptr(self.rxbuf.as_mut_ptr());
        }

        // Prepare RRAM before arming RX so the completion IRQ can wake quickly.
        #[cfg(all(feature = "nrf54lm20", feature = "reduce-rram-latency"))]
        {
            embassy_nrf::pac::RRAMC
                .power()
                .lowpowerconfig()
                .modify(|w| {
                    w.set_mode(embassy_nrf::pac::rramc::vals::Mode::Standby);
                });
            embassy_nrf::pac::RRAMC.tasks_wakeup().write_value(1);
        }

        ppi.configure_peripheral_next_rx(clock, egu);
        egu.select_interrupt(TimeslotEnd);

        self.timing.is_hb = loop_diff % self.timing.hb_loops == 0;

        #[cfg(feature = "defmt")]
        {
            let ts = clock.get_start_ts().unscaled();
            let cc = clock.get_start_hw();
            debug!(
                "start ts {} cc {} timeout {} slot {} ch {} is_hb {}",
                ts, cc, _timeout, self.timing.slot_now, channel, self.timing.is_hb
            );
        }
    }

    fn disabled_rx(&mut self) {
        let DeviceContext {
            radio,
            clock,
            ppi,
            egu,
            context: ctx,
            ..
        } = &mut self.device;
        let rxbuf = &mut self.rxbuf;
        let retry = &mut self.slot.last_retry;
        let link = &mut self.slot.link;
        let tm = &mut self.timing;
        let ep = CHANNELS.link_ep(0);

        #[cfg(all(feature = "nrf54lm20", feature = "reduce-rram-latency"))]
        {
            embassy_nrf::pac::RRAMC
                .power()
                .lowpowerconfig()
                .modify(|w| {
                    w.set_mode(embassy_nrf::pac::rramc::vals::Mode::PowerOff);
                });
        }

        let events = radio.take_interrupt_events();
        let rx_failed = !events.crc_ok;
        // Both backends keep TX in TXIDLE until ACK setup. TXREADY may
        // legitimately precede this handler and is not an RX failure.
        if rx_failed {
            // do some timeout handle here

            debug!(
                "rx failed: crcok {} rxready {} txready {}",
                events.crc_ok, events.rxready, events.txready
            );

            let rd = embassy_nrf::pac::RADIO;

            debug!(
                "rx failed: slot {} address {} end {} crcerror {}",
                tm.slot_now,
                rd.events_address().read() != 0,
                rd.events_end().read() != 0,
                rd.events_crcerror().read() != 0,
            );

            rd.events_address().write_value(0);
            rd.events_end().write_value(0);
            rd.events_crcerror().write_value(0);

            // we must check next slot
            tm.check_next_slot = true;

            if retry.is_timeout(tm.slot_now) {
                debug!("Rx failed resync");
                link.state = Disconnected;
                ep.conn.signal(Disconnected);

                self.resync();
            } else {
                radio.disable();
                self.prepare_next_rx();
            }

            return;
        }

        let my_sn = link.sn;
        let my_nesn = link.nesn;
        let ur_sn = rxbuf.pdu1.sn();
        let ur_nesn = rxbuf.pdu1.nesn();
        let rx_packet_type = rxbuf.pdu1.packet_type();

        let rx_md = rxbuf.pdu1.md();

        // Central sent not requested sn, so retransmit payload
        let is_retransmit_payload = my_nesn != ur_sn;

        let Ok((header, body)) = rxbuf.read_header::<Self>() else {
            debug!("invalid rx header");

            tm.check_next_slot = true;

            if retry.is_timeout(tm.slot_now) {
                link.state = Disconnected;
                ep.conn.signal(Disconnected);
                self.resync();
            } else {
                radio.disable();
                self.prepare_next_rx();
            }

            return;
        };

        #[cfg(feature = "defmt")]
        let mut log_sync_after_ack = false;
        let rx_outcome: Option<(CurrentRx, &[u8])> = 'rx_retry: {
            if is_retransmit_payload {
                break 'rx_retry None;
            }

            let slice = 'rx_skip: {
                match rx_packet_type {
                    PacketType::Control => {
                        let Some((&byte, _data)) = body.as_slice().split_first() else {
                            break 'rx_skip None;
                        };

                        let Ok(command) = ControlCommand::try_from(byte) else {
                            break 'rx_skip None;
                        };

                        match command {
                            ControlCommand::Sync => {
                                // noop, for now
                                #[cfg(feature = "defmt")]
                                {
                                    log_sync_after_ack = true;
                                }
                            }
                        }

                        None
                    }
                    PacketType::Start => {
                        let Some((&total_len, data)) = body.as_slice().split_first() else {
                            // debug!("missing packet length, drop");
                            link.current_rx = None;
                            break 'rx_skip None;
                        };

                        if total_len == 0 {
                            // debug!("empty packet");
                            link.current_rx = None;
                            break 'rx_retry None;
                        }

                        let rx_packet = link
                            .current_rx
                            .take()
                            .or_else(|| ep.rx.try_acquire().map(CurrentPacket::new));

                        let Some(mut rx_packet) = rx_packet else {
                            // debug!("buf get fail, retry");
                            break 'rx_retry None;
                        };

                        rx_packet.clear();

                        if rx_packet.set_total_len(total_len as usize).is_err() {
                            break 'rx_skip None;
                        }

                        Some((rx_packet, data))
                    }
                    PacketType::Continue => {
                        let data = body.as_slice();

                        if data.is_empty() {
                            debug!("Continue data must not be empty!");
                            link.current_rx = None;

                            break 'rx_skip None;
                        }

                        let Some(rx_packet) = link.current_rx.take() else {
                            debug!("we must have packet here! skip this packet");
                            break 'rx_skip None;
                        };

                        Some((rx_packet, body.as_slice()))
                    }

                    PacketType::Invalid => {
                        debug!("data has been corrupted, skip this packet");

                        None
                    }
                }
            };

            link.nesn = !link.nesn;
            slice
        };

        let txbuf = &mut self.slot.txbuf;

        // central requested same nesn, so tx failed
        let tx_failed = my_sn == ur_nesn && self.slot.pkind != PacketKind::Empty;

        let is_new_packet_tx = if tx_failed {
            let slot_now = tm.slot_now;
            if retry.is_timeout(slot_now) {
                debug!("Tx failed resync");
                link.state = Disconnected;
                ep.conn.signal(Disconnected);
                self.resync();
                return;
            }

            link.tx_attempts = link.tx_attempts.saturating_add(1);
            if link.tx_attempts & 3 == 3 {
                self.slot.tx_power.raise();
            }

            link.current_tx.is_none()
        } else {
            if self.slot.pkind != PacketKind::Empty {
                link.sn = !link.sn;
            }

            link.tx_attempts = 0;
            retry.clear();

            if let Some(packet) = link.current_tx.as_mut() {
                if packet.commit_fragment() == 0 {
                    link.current_tx = None;
                }
            }
            true
        };

        // we have to send new (sub)packet
        if is_new_packet_tx {
            txbuf.clear();

            let up_header = UpHeader::new();

            let mut tx_body = txbuf.write_header::<Self>(up_header);

            let (ptype, more) = if let Some(packet) = link.current_tx.as_mut() {
                tx_body.write_bytes(packet.get_fragment(tx_body.free_size()));

                self.slot.pkind = PacketKind::Data;

                (PacketType::Continue, packet.remain_after_tx() > 0)
            } else if let Some(phandle) = ep.tx.try_receive() {
                let mut packet = CurrentPacket::new(phandle);

                tx_body.write_bytes(&[packet.total_len() as u8]);
                tx_body.write_bytes(packet.get_fragment(tx_body.free_size()));

                let is_more = packet.remain_after_tx() > 0;
                self.slot.pkind = PacketKind::Data;

                link.current_tx = Some(packet);

                (PacketType::Start, is_more)
            } else {
                tx_body.write_bytes(&[0u8; RADIO_RSSI_PADDING_BYTES]);
                self.slot.pkind = PacketKind::Empty;

                (PacketType::Start, false)
            };

            txbuf.pdu1.set_packet_type(ptype);
            txbuf.pdu1.set_md(more);
            tm.check_next_slot = (tx_failed && tm.check_next_slot) // resume previous pending state
                || more
                ||rx_md
                || tm.is_hb;
        }

        if self.slot.tx_power.update(header.rssi) {
            radio.set_tx_power(self.slot.tx_power.get());
        }

        txbuf.pdu1.set_sn(link.sn);
        txbuf.pdu1.set_nesn(link.nesn);

        unsafe {
            radio.set_packetptr(txbuf.as_mut_ptr());
        }

        // after tx

        // set ppi here
        ppi.configure_peripheral_tx_ack(egu);
        egu.select_interrupt(TimeslotEnd);

        self.state = PeripheralState::TxAck;

        #[cfg(feature = "defmt")]
        if log_sync_after_ack {
            error!("peripheral should not send this");
        }

        // sync stuff
        if clock.update_sync(tm) == UpdateResult::Updated {
            ctx.refslot = tm.slot_now;
        }

        // do some rx handle stuff here, after we set tx
        let Some((mut rx_packet, slice)) = rx_outcome else {
            return;
        };

        if rx_packet.append_from_slice(slice).is_err() {
            error!("this shouldn't be happening, so we dump packet");

            return;
        }

        debug!(
            "tx: type {} len {} body {:x}",
            txbuf.pdu1.packet_type(),
            txbuf.len(),
            txbuf.data()
        );

        debug!(
            "rx: type {} len {} body {:x}",
            rx_packet_type,
            slice.len(),
            slice
        );

        debug!(
            "rx: current {{total {} copied {}}}",
            rx_packet.total_len(),
            rx_packet.copied()
        );

        debug!(
            "rx: {{sn {} ne {}}} tx: {{sn {} ne {}}} rssi: {}, txpower: {}, next_slot: {}",
            ur_sn,
            my_nesn,
            my_sn,
            ur_nesn,
            -(header.rssi as i8),
            (self.slot.tx_power.get().to_bits() as i8),
            tm.check_next_slot
        );

        if rx_packet.is_ongoing() {
            link.current_rx = Some(rx_packet);
            return;
        }

        ep.rx.send(rx_packet.packet());
    }

    fn disabled_tx_ack(&mut self) {
        let _ = self.device.radio.take_interrupt_events();

        if self.timing.check_next_slot {
            self.prepare_next_rx();
        } else {
            self.prepare_rx();
        }
    }

    pub fn session_endpoints<const W: usize>() -> [YipSessionEndpoints; W] {
        assert!(W <= YIP_MAX_LINK_COUNT);

        core::array::from_fn(|i| CHANNELS.session_ep(i))
    }
}
