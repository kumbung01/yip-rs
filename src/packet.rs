use core::ops::{Deref, DerefMut};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use crate::config::*;
use crate::link::LinkState;
use crate::linkedlist::*;

pub(crate) type ConnSignal = Signal<CriticalSectionRawMutex, LinkState>;
pub(crate) struct ConnSignaler {
    pub(crate) signal: &'static ConnSignal,
}

pub(crate) struct ConnWaiter {
    pub(crate) signal: &'static ConnSignal,
}

impl ConnSignaler {
    pub(crate) fn signal(&self, state: LinkState) {
        self.signal.signal(state);
    }
}

impl ConnWaiter {
    pub(crate) async fn wait(&self) -> LinkState {
        self.signal.wait().await
    }
}

static_pool!(
    pub TxPacketPool: TxPacketStorage,
    init: TxPool::new([const {core::cell::UnsafeCell::new(Packet::new())}; YIP_POOL_SIZE]),
);

static_pool!(
    pub RxPacketPool: RxPacketStorage,
    init: RxPool::new([const {core::cell::UnsafeCell::new(Packet::new())}; YIP_POOL_SIZE]),
);

type TxPacketStorage = TxPool<Packet, CriticalSectionRawMutex, YIP_POOL_SIZE, YIP_MAX_LINK_COUNT>;
type RxPacketStorage = RxPool<Packet, CriticalSectionRawMutex, YIP_POOL_SIZE>;

pub struct PacketHandle(pub(crate) ItemHandle<RxPacketPool>);

impl Deref for PacketHandle {
    type Target = Packet;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for PacketHandle {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

pub(crate) type TxList = PoolLinkedList<TxPacketPool, CriticalSectionRawMutex>;
pub(crate) type RxList = PoolLinkedList<RxPacketPool, CriticalSectionRawMutex>;

pub(crate) type TxSender = Sender<'static, TxList>;
pub(crate) type TxReceiver = Receiver<'static, TxList>;
pub(crate) type RxSender = Sender<'static, RxList>;
pub(crate) type RxReceiver = Receiver<'static, RxList>;

pub struct YipSessionEndpoints {
    pub(crate) conn: ConnWaiter,
    pub(crate) rx: RxReceiver,
    pub(crate) tx: TxSender,
}

pub struct YipLinkEndpoints {
    pub(crate) conn: ConnSignaler,
    pub(crate) rx: RxSender,
    pub(crate) tx: TxReceiver,
}

impl YipLinkEndpoints {
    pub(crate) fn clear(&self) {
        self.rx.queue.clear();
        self.tx.queue.clear();
    }
}

pub struct PacketChannels<const W: usize> {
    conn: [ConnSignal; W],
    rx: [RxList; W],
    tx: [TxList; W],
}

impl<const W: usize> PacketChannels<W> {
    pub const fn new() -> Self {
        Self {
            conn: [const { ConnSignal::new() }; W],
            rx: [const { RxList::new() }; W],
            tx: [const { TxList::new() }; W],
        }
    }

    pub fn session_ep(&'static self, i: usize) -> YipSessionEndpoints {
        YipSessionEndpoints {
            conn: ConnWaiter {
                signal: &self.conn[i],
            },
            rx: self.rx[i].receiver(),
            tx: self.tx[i].sender(),
        }
    }

    pub fn link_ep(&'static self, n: usize) -> YipLinkEndpoints {
        YipLinkEndpoints {
            conn: ConnSignaler {
                signal: &self.conn[n],
            },
            rx: self.rx[n].sender(),
            tx: self.tx[n].receiver(),
        }
    }
}

pub struct Packet {
    len: u8,
    data: [u8; YIP_PACKET_SIZE],
}

impl Packet {
    const fn new() -> Self {
        Self {
            len: 0,
            data: [0; YIP_PACKET_SIZE],
        }
    }

    pub fn set(&mut self, data: &[u8]) -> Result<(), ()> {
        if data.len() > YIP_PACKET_SIZE {
            return Err(());
        }

        self.len = data.len() as u8;
        self.data[..data.len()].copy_from_slice(data);

        Ok(())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

impl Reset for Packet {
    fn clear(&mut self) {
        self.len = 0;
    }
}

impl AsRef<[u8]> for Packet {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

pub(crate) struct CurrentPacket<P: StaticPool> {
    packet: ItemHandle<P>,
    copied: usize,
    in_flight_len: usize,
}

pub(crate) type CurrentTx = CurrentPacket<TxPacketPool>;
pub(crate) type CurrentRx = CurrentPacket<RxPacketPool>;

impl<P> CurrentPacket<P>
where
    P: StaticPool,
    P::Storage: PoolAccess<Item = Packet>,
{
    pub(crate) const fn new(packet: ItemHandle<P>) -> Self {
        Self {
            packet,
            copied: 0,
            in_flight_len: 0,
        }
    }

    pub(crate) fn is_new(&self) -> bool {
        self.copied == 0
    }

    pub(crate) fn get_fragment(&mut self, max_len: usize) -> &[u8] {
        let data = self.packet.as_ref();
        let start = self.copied.min(data.len());
        let end = start.saturating_add(max_len).min(data.len());

        self.in_flight_len = end - start;

        &data[start..end]
    }

    pub(crate) fn commit_fragment(&mut self) -> usize {
        let total_len = self.total_len();

        self.copied = self
            .copied
            .saturating_add(self.in_flight_len)
            .min(total_len);

        self.in_flight_len = 0;

        total_len - self.copied
    }

    pub(crate) fn append_from_slice(&mut self, slice: &[u8]) -> Result<(), ()> {
        let end = self.copied.checked_add(slice.len()).ok_or(())?;
        if end > self.total_len() {
            debug!("append failed: total: {}, end: {}", self.packet.len, end);
            return Err(());
        }

        self.packet.data[self.copied..end].copy_from_slice(slice);
        self.copied = end;
        Ok(())
    }

    pub(crate) fn packet(self) -> ItemHandle<P> {
        self.packet
    }

    pub(crate) fn total_len(&self) -> usize {
        self.packet.len as usize
    }

    // #[cfg(any(feature = "defmt", feature = "log"))]
    // pub(crate) fn copied(&self) -> usize {
    // self.copied
    // }

    pub(crate) fn remaining_len(&self) -> usize {
        self.packet.as_slice().len() - self.copied
    }

    pub(crate) fn remain_before_tx(&self) -> usize {
        self.packet.as_slice().len() - self.copied
    }

    pub(crate) fn remain_after_tx(&self) -> usize {
        self.remain_before_tx() - self.in_flight_len
    }

    pub(crate) fn is_ongoing(&self) -> bool {
        assert!(
            self.total_len() >= self.copied,
            "total length must be >= copied"
        );
        self.total_len() != self.copied
    }

    pub(crate) fn clear(&mut self) {
        self.packet.clear();
        self.copied = 0;
        self.in_flight_len = 0;
    }

    pub(crate) fn set_total_len(&mut self, total_len: usize) -> Result<(), ()> {
        // we don't panic here, since data can be corrupted
        if total_len > YIP_PACKET_SIZE {
            debug!("invalid packet size({})", total_len);
            return Err(());
        }

        self.packet.len = u8::try_from(total_len).map_err(|_| ())?;

        Ok(())
    }
}
