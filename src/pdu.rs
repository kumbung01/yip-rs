use bitfield_struct::{bitenum, bitfield};

#[bitfield(u8)]
pub(crate) struct PduByte0 {
    #[bits(6)]
    pub length: u8,

    #[bits(2)]
    __: u8,
}

// packetbuffer.rs
impl PduByte0 {
    pub(crate) const LENGTH_BIT_COUNT: usize = Self::LENGTH_BITS;
}

#[bitfield(u8)]
pub(crate) struct PduByte1 {
    pub sn: bool,
    pub nesn: bool,

    #[bits(2)]
    pub packet_type: PacketType,
    pub md: bool,

    #[bits(3)]
    pub reserved: u8,
}

impl PduByte1 {
    pub const S1_BITS: u8 = 8 - PduByte1::RESERVED_BITS as u8;
}

#[bitenum]
#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum PacketType {
    Start = 0,
    Continue = 1,
    Control = 2,

    #[fallback]
    Invalid = 7,
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum PacketKind {
    Empty,
    Data,
    Control,
}
