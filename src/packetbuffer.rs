use core::marker::PhantomData;

use crate::config::*;
use crate::header::{DownHeader, UpHeader, YipHeader, YipPayload, YipRoleHeader};
use crate::pdu::{PduByte0, PduByte1};
use crate::role::YipRoleStack;

#[repr(C)]
pub(crate) struct YipPacketBuffer<R> {
    pdu0: PduByte0,
    pub(crate) pdu1: PduByte1,
    data: [u8; YIP_SUB_PACKET_SIZE],

    _role: PhantomData<R>,
}

impl<R: YipRoleHeader> YipPacketBuffer<R> {
    pub(crate) fn write_header(&mut self, header: R::TxHeader) -> Result<BodyWriter<'_, R>, ()> {
        self.write(header)?;
        Ok(BodyWriter { buffer: self })
    }

    pub(crate) fn read_header(&mut self) -> Result<(R::RxHeader, BodyReader<'_, R>), ()> {
        let mut header = R::RxHeader::new();
        let received = self.data.get(..self.len()).ok_or(())?;
        let bytes = received.get(..R::RxHeader::SIZE).ok_or(())?;

        if header.copy(bytes) != R::RxHeader::SIZE {
            return Err(());
        }

        Ok((
            header,
            BodyReader {
                buffer: self,
                start: R::RxHeader::SIZE,
            },
        ))
    }
}

impl<R> YipPacketBuffer<R> {
    pub(crate) const fn new() -> Self {
        Self {
            pdu0: PduByte0::new(),
            pdu1: PduByte1::new(),
            data: [0; YIP_SUB_PACKET_SIZE],

            _role: PhantomData,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.pdu0.length() as usize
    }

    pub(crate) fn free_size(&self) -> usize {
        YIP_SUB_PACKET_SIZE - self.pdu0.length() as usize
    }

    fn write<T: YipPayload>(&mut self, value: T) -> Result<(), ()> {
        let start = self.pdu0.length() as usize;
        let end = start.checked_add(value.encoded_len()).ok_or(())?;

        let destination = self.data.get_mut(start..end).ok_or(())?;
        value.encode_into(destination);

        self.pdu0.set_length(end as u8);
        Ok(())
    }

    fn read(&self, dst: &mut [u8], start: usize, end: usize) -> Result<(), ()> {
        dst.copy_from_slice(&self.data[start..end]);
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        self.pdu0.set_length(0);
        self.pdu1.set_more(false);
    }

    pub(crate) fn as_ptr(&self) -> *const u8 {
        core::ptr::from_ref(self).cast()
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut u8 {
        core::ptr::from_mut(self).cast()
    }
}

pub(crate) struct BodyReader<'a, R> {
    buffer: &'a mut YipPacketBuffer<R>,
    start: usize,
}

impl<'a, R: YipRoleHeader> BodyReader<'a, R> {
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.buffer.data[self.start..self.buffer.len()]
    }
}

pub(crate) struct BodyWriter<'a, R> {
    buffer: &'a mut YipPacketBuffer<R>,
}

impl<'a, R> BodyWriter<'a, R> {
    pub(crate) fn write<T: YipPayload>(&mut self, value: T) -> Result<(), ()> {
        self.buffer.write(value)
    }

    pub(crate) fn free_size(&mut self) -> usize {
        self.buffer.free_size()
    }
}

pub(crate) struct YipControlPacket {
    pub(crate) refslot: u32,
    pub(crate) slotsize_ticks: u32,
    pub(crate) pipes: u8,
}

impl YipPayload for YipControlPacket {
    fn encoded_len(&self) -> usize {
        use core::mem::size_of_val;

        size_of_val(&self.refslot) + size_of_val(&self.slotsize_ticks) + size_of_val(&self.pipes)
    }

    fn encode_into(self, dst: &mut [u8]) {
        dst[0..4].copy_from_slice(&self.refslot.to_le_bytes());
        dst[4..8].copy_from_slice(&self.slotsize_ticks.to_le_bytes());
        dst[8] = self.pipes;
    }
}

impl<R: YipRoleHeader> TryFrom<BodyReader<'_, R>> for YipControlPacket {
    type Error = ();

    fn try_from(reader: BodyReader<'_, R>) -> Result<Self, Self::Error> {
        let bytes: &[u8; 9] = reader.as_slice().try_into().map_err(|_| ())?;

        Ok(Self {
            refslot: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            slotsize_ticks: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            pipes: bytes[8],
        })
    }
}
