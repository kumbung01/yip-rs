use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use crate::link::LinkState;
use crate::linkedlist::{
    ItemHandle, PoolLinkedList, PoolStorage, PoolView, Receiver, Reset, Sender,
};

use crate::config::YIP_PACKET_SIZE;

pub(crate) type ConnSignal = Signal<CriticalSectionRawMutex, LinkState>;
pub(crate) struct ConnSignaler<'a> {
    signal: &'a ConnSignal,
}

pub(crate) struct ConnWaiter<'a> {
    signal: &'a ConnSignal,
}

impl<'a> ConnSignaler<'a> {
    pub(crate) fn signal(&self, state: LinkState) {
        self.signal.signal(state);
    }
}

impl<'a> ConnWaiter<'a> {
    pub(crate) async fn wait(&self) -> LinkState {
        self.signal.wait().await
    }
}

pub type PacketPoolStorage<const S: usize, const W: usize> =
    PoolStorage<Packet, CriticalSectionRawMutex, S, W>;

impl<const S: usize, const W: usize> PacketPoolStorage<S, W> {
    pub fn new_packet() -> Self {
        Self::new(Packet::new)
    }
}

pub type PacketPoolView<'p, const W: usize> = PoolView<'p, Packet, CriticalSectionRawMutex, W>;
pub type PacketHandle<'p, const W: usize> = ItemHandle<'p, Packet, CriticalSectionRawMutex, W>;
pub(crate) type PacketLinkedList<'p, const W: usize> =
    PoolLinkedList<'p, Packet, CriticalSectionRawMutex, W>;
pub(crate) type PacketSender<'q, 'p, const W: usize> =
    Sender<'q, 'p, Packet, CriticalSectionRawMutex, W>;
pub(crate) type PacketReceiver<'q, 'p, const W: usize> =
    Receiver<'q, 'p, Packet, CriticalSectionRawMutex, W>;

pub struct YipLinkEndpoints<'q, 'p, const W: usize> {
    pub(crate) conn: ConnSignaler<'q>,
    pub(crate) rx: PacketSender<'q, 'p, W>,
    pub(crate) tx: PacketReceiver<'q, 'p, W>,
}

pub struct YipSessionEndpoints<'q, 'p, const W: usize> {
    pub(crate) conn: ConnWaiter<'q>,
    pub(crate) rx: PacketReceiver<'q, 'p, W>,
    pub(crate) tx: PacketSender<'q, 'p, W>,
}

pub struct PacketChannel<'p, const W: usize> {
    conn: ConnSignal,
    rx: PacketLinkedList<'p, W>,
    tx: PacketLinkedList<'p, W>,
}

impl<'p, const W: usize> PacketChannel<'p, W> {
    pub fn new(pool: &'p PacketPoolView<'p, W>) -> Self {
        Self {
            conn: ConnSignal::new(),
            rx: PoolLinkedList::new(pool),
            tx: PoolLinkedList::new(pool),
        }
    }

    pub fn split(&mut self) -> (YipLinkEndpoints<'_, 'p, W>, YipSessionEndpoints<'_, 'p, W>) {
        (
            YipLinkEndpoints {
                conn: ConnSignaler { signal: &self.conn },
                rx: self.rx.sender(),
                tx: self.tx.receiver(),
            },
            YipSessionEndpoints {
                conn: ConnWaiter { signal: &self.conn },
                rx: self.rx.receiver(),
                tx: self.tx.sender(),
            },
        )
    }
}

pub struct PacketChannels<'p, const W: usize> {
    channels: [PacketChannel<'p, W>; W],
}

impl<'p, const W: usize> PacketChannels<'p, W> {
    pub fn new(pool: &'p PacketPoolView<'p, W>) -> Self {
        Self {
            channels: core::array::from_fn(|_| PacketChannel::new(pool)),
        }
    }

    pub fn endpoints(
        &mut self,
    ) -> (
        [YipLinkEndpoints<'_, 'p, W>; W],
        [YipSessionEndpoints<'_, 'p, W>; W],
    ) {
        let mut pairs = self.channels.each_mut().map(|channel| {
            let (link, session) = channel.split();
            (Some(link), Some(session))
        });

        let links = core::array::from_fn(|i| pairs[i].0.take().unwrap());
        let sessions = core::array::from_fn(|i| pairs[i].1.take().unwrap());

        (links, sessions)
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

    pub(crate) fn set(&mut self, data: &[u8]) -> Result<(), ()> {
        if data.len() > YIP_PACKET_SIZE {
            return Err(());
        }

        self.len = data.len() as u8;
        self.data[..data.len()].copy_from_slice(data);

        Ok(())
    }

    pub(crate) fn append(&mut self, data: &[u8]) -> Result<(), ()> {
        let start = self.len as usize;
        let end = start.checked_add(data.len()).ok_or(())?;

        let destination = self.data.get_mut(start..end).ok_or(())?;
        let new_len = u8::try_from(end).map_err(|_| ())?;

        destination.copy_from_slice(data);
        self.len = new_len;

        Ok(())
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

impl Reset for Packet {
    fn clear(&mut self) {
        self.len = 0;
    }
}

pub(crate) struct CurrentPacket<'p, const W: usize> {
    packet: PacketHandle<'p, W>,
    copied: usize,
    in_flight_len: usize,
}

impl<'p, const W: usize> CurrentPacket<'p, W> {
    pub(crate) const fn new(packet: PacketHandle<'p, W>) -> Self {
        Self {
            packet: packet,
            copied: 0,
            in_flight_len: 0,
        }
    }

    pub(crate) fn get_fragment(&mut self, max_len: usize) -> &[u8] {
        let data = self.packet.as_slice();
        let start = self.copied.min(data.len());
        let end = start.saturating_add(max_len).min(data.len());

        self.in_flight_len = end - start;

        &data[start..end]
    }

    pub(crate) fn commit_fragment(&mut self) -> usize {
        let total_len = self.packet.as_slice().len();

        self.copied = self
            .copied
            .saturating_add(self.in_flight_len)
            .min(total_len);

        self.in_flight_len = 0;

        total_len - self.copied
    }

    #[allow(dead_code)]
    pub(crate) fn copy_from_slice(&mut self, slice: &[u8]) -> Result<(), ()> {
        self.packet.set(slice)
    }

    pub(crate) fn append_from_slice(&mut self, slice: &[u8]) -> Result<(), ()> {
        self.packet.append(slice)
    }

    pub(crate) fn packet(self) -> PacketHandle<'p, W> {
        self.packet
    }

    pub(crate) fn remaining_len(&self) -> usize {
        self.packet.as_slice().len() - self.copied
    }

    pub(crate) fn is_more(&self) -> bool {
        self.remaining_len() > self.in_flight_len
    }
}
