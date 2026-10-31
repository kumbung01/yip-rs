use crate::link::LinkState;
use crate::packet::{ConnWaiter, PacketHandle, RxReceiver, TxSender, YipSessionEndpoints};

pub struct YipSession {
    reader: YipReader,
    writer: YipWriter,
    conn: YipConnectionListener,
}

impl YipSession {
    pub fn new(endpoints: YipSessionEndpoints) -> Self {
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

    pub fn split(self) -> (YipReader, YipWriter, YipConnectionListener) {
        (self.reader, self.writer, self.conn)
    }
}

pub struct YipReader {
    receiver: RxReceiver,
}

impl YipReader {
    pub async fn read(&self) -> PacketHandle {
        PacketHandle(self.receiver.receive().await)
    }
}

pub struct YipWriter {
    sender: TxSender,
}

impl YipWriter {
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

pub struct YipConnectionListener {
    waiter: ConnWaiter,
}

impl<'s> YipConnectionListener {
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
            if matches!(self.waiter.wait().await, LinkState::Connected(_)) {
                break;
            }
        }
    }
}
