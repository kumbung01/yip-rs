use crate::config::*;
use crate::payload::Payload;
use crate::pdu::{PduByte0, PduByte1};
use crate::role::private::RoleHeader;

#[repr(C)]
pub(crate) struct PacketBuffer {
    pdu0: PduByte0,
    pub(crate) pdu1: PduByte1,
    data: [u8; YIP_SUB_PACKET_SIZE],
}

impl PacketBuffer {
    #[inline(always)]
    pub(crate) fn write_header<R: RoleHeader>(&mut self, header: R::TxHeader) -> BodyWriter<'_> {
        self.write(header);
        BodyWriter { buffer: self }
    }

    pub(crate) fn read_header<R: RoleHeader>(
        &mut self,
    ) -> Result<(R::RxHeader, BodyReader<'_>), ()> {
        let received = self.data.get(..self.len()).ok_or(())?;
        let bytes = received.get(..R::RxHeader::SIZE).ok_or(())?;
        let header = R::RxHeader::decode(bytes)?;

        Ok((
            header,
            BodyReader {
                buffer: self,
                start: R::RxHeader::SIZE,
            },
        ))
    }

    #[inline(always)]
    pub(crate) fn set_len(&mut self, len: usize) {
        self.pdu0.set_length(len as u8);
    }
}

impl PacketBuffer {
    pub(crate) const fn new() -> Self {
        Self {
            pdu0: PduByte0::new(),
            pdu1: PduByte1::new(),
            data: [0; YIP_SUB_PACKET_SIZE],
        }
    }

    #[inline(always)]
    pub(crate) fn len(&self) -> usize {
        self.pdu0.length() as usize
    }

    #[inline(always)]
    pub(crate) fn free_size(&self) -> usize {
        YIP_SUB_PACKET_SIZE - self.pdu0.length() as usize
    }

    #[inline(always)]
    fn write<T: Payload>(&mut self, value: T) {
        let start = self.pdu0.length() as usize;
        let end = start + T::SIZE;
        assert!(
            end <= YIP_SUB_PACKET_SIZE,
            "end must be smaller than sub packet size"
        );

        // SAFETY: The capacity check above covers the complete payload, including every field.
        let dst = unsafe {
            core::slice::from_raw_parts_mut(self.data.as_mut_ptr().add(start), T::SIZE)
        };
        // The slice length equals SIZE, so encode's entry check is statically satisfied.
        value.encode(dst);
        self.pdu0.set_length(end as u8);
    }

    pub(crate) fn clear(&mut self) {
        self.pdu0.set_length(0);
        self.pdu1.set_packet_type(crate::pdu::PacketType::Start);
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut u8 {
        core::ptr::from_mut(self).cast()
    }

    // #[cfg(any(feature = "defmt", feature = "log"))]
    // pub(crate) fn data(&self) -> &[u8] {
    // &self.data[0..self.len()]
    // }
}

pub(crate) struct BodyReader<'a> {
    buffer: &'a mut PacketBuffer,
    start: usize,
}

impl<'a> BodyReader<'a> {
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.buffer.data[self.start..self.buffer.len()]
    }
}

pub(crate) struct BodyWriter<'a> {
    buffer: &'a mut PacketBuffer,
}

impl<'a> BodyWriter<'a> {
    #[inline(always)]
    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        let start = self.buffer.len();
        let end = start + bytes.len();

        self.buffer.data[start..end].copy_from_slice(bytes);
        self.buffer.set_len(end);
    }

    #[inline(always)]
    pub(crate) fn write<T: Payload>(&mut self, value: T) {
        self.buffer.write(value);
    }

    #[inline(always)]
    pub(crate) fn free_size(&self) -> usize {
        self.buffer.free_size()
    }
}
