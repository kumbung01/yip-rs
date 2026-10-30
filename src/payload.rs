pub use yip_macros::Payload;

/// Fixed wire size; fields are encoded in declaration order, little endian.
/// Encoding requires at least SIZE destination bytes; decoding requires exactly SIZE.
pub trait Payload: Sized {
    const SIZE: usize;
    fn encode(self, dst: &mut [u8]);
    fn decode(src: &[u8]) -> Result<Self, ()>;
}

pub trait Header: Payload {}

macro_rules! integer_payload {
    ($($ty:ty),* $(,)?) => {$(
        impl Payload for $ty {
            const SIZE: usize = core::mem::size_of::<Self>();
            fn encode(self, dst: &mut [u8]) {
                dst[..Self::SIZE].copy_from_slice(&self.to_le_bytes());
            }
            fn decode(src: &[u8]) -> Result<Self, ()> {
                Ok(Self::from_le_bytes(src.try_into().map_err(|_| ())?))
            }
        }
    )*};
}

// Keep wire sizes independent of the target architecture: no usize/isize.
integer_payload!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

impl<const N: usize> Payload for [u8; N] {
    const SIZE: usize = N;
    fn encode(self, dst: &mut [u8]) {
        dst[..N].copy_from_slice(&self);
    }
    fn decode(src: &[u8]) -> Result<Self, ()> {
        src.try_into().map_err(|_| ())
    }
}

#[derive(Payload)]
pub struct DownHeader {
    pub(crate) rssi: u8,
}
impl Header for DownHeader {}

#[derive(Payload)]
pub struct UpHeader {
    pub(crate) _dummy: [u8; 0],
}
impl UpHeader {
    pub(crate) fn new() -> Self {
        Self { _dummy: [] }
    }
}
impl Header for UpHeader {}

#[derive(Payload)]
pub(crate) struct SyncPacket {
    pub(crate) refslot: u32,
    pub(crate) slot_ticks: u32,
    pub(crate) pipes: u8,
}

#[repr(u8)]
#[derive(Payload, Clone, Copy, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum ControlCommand {
    Sync,
}

impl TryFrom<u8> for ControlCommand {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::decode(&[value])
    }
}

#[derive(Payload)]
pub(crate) struct ControlHeader {
    pub(crate) command: ControlCommand,
}
