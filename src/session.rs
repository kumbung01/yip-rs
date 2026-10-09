use crate::link::LinkState;
use crate::packet::{ConnWaiter, PacketHandle, PacketReceiver, PacketSender, YipSessionEndpoints};

pub struct YipSession<'q, 'p, const W: usize> {
    reader: YipReader<'q, 'p, W>,
    writer: YipWriter<'q, 'p, W>,
    conn: YipConnectionListener<'q>,
}

impl<'q, 'p, const W: usize> YipSession<'q, 'p, W> {
    pub fn new(endpoints: YipSessionEndpoints<'q, 'p, W>) -> Self {
        Self {
            reader: YipReader {
                receiver: endpoints.rx,
            },
            writer: YipWriter {
                sender: endpoints.tx,
            },
            conn: YipConnectionListener {
                waiter: endpoints.conn,
            },
        }
    }

    pub fn split(
        self,
    ) -> (
        YipReader<'q, 'p, W>,
        YipWriter<'q, 'p, W>,
        YipConnectionListener<'q>,
    ) {
        (self.reader, self.writer, self.conn)
    }
}

pub struct YipReader<'q, 'p, const W: usize> {
    receiver: PacketReceiver<'q, 'p, W>,
}

impl<'q, 'p, const W: usize> YipReader<'q, 'p, W> {
    pub async fn read(&self) -> PacketHandle<'p, W> {
        self.receiver.receive().await
    }
}

pub struct YipWriter<'q, 'p, const W: usize> {
    sender: PacketSender<'q, 'p, W>,
}

impl<'q, 'p, const W: usize> YipWriter<'q, 'p, W> {
    pub fn try_write(&mut self, data: &[u8]) -> Result<(), ()> {
        let mut packet = self.sender.try_acquire().ok_or(())?;

        packet.set(data)?;
        self.sender.send(packet);

        crate::trigger_start();
        Ok(())
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), ()> {
        let mut packet = self.sender.acquire().await.map_err(|_| ())?;

        packet.set(data)?;
        self.sender.send(packet);

        crate::trigger_start();
        Ok(())
    }
}

pub struct YipConnectionListener<'q> {
    waiter: ConnWaiter<'q>,
}

impl<'q> YipConnectionListener<'q> {
    pub async fn wait(&self) -> LinkState {
        self.waiter.wait().await
    }

    pub async fn wait_disconnected(&self) {
        loop {
            if self.waiter.wait().await == LinkState::Disconnected {
                break;
            }
        }
    }

    pub async fn wait_connected(&self) {
        loop {
            if self.waiter.wait().await == LinkState::Connected {
                break;
            }
        }
    }
}
