use bitfield_struct::bitfield;

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
    pub ctrl: bool,
    pub more: bool,
    pub sn: bool,
    pub nesn: bool,

    #[bits(4)]
    __: u8,
}

impl PduByte1 {
    pub const S1_BITS: u8 = PduByte1::CTRL_BITS as u8
        + PduByte1::MORE_BITS as u8
        + PduByte1::SN_BITS as u8
        + PduByte1::NESN_BITS as u8;
}
