pub use yip_macros::Payload;

/// Fixed wire size; fields are encoded in declaration order, little endian.
/// Encoding requires at least SIZE destination bytes; decoding requires exactly SIZE.
pub trait Payload: Sized {
    const SIZE: usize;
    #[inline(always)]
    fn encode(self, dst: &mut [u8]) {
        debug_assert!(dst.len() >= Self::SIZE, "payload destination too short");
        let dst = &mut dst[..Self::SIZE];
        // SAFETY: The complete encoded payload fits in the destination.
        unsafe { self.encode_unchecked(dst.as_mut_ptr()) };
    }

    /// Writes exactly SIZE bytes in wire format without checking the destination.
    ///
    /// # Safety
    /// `dst` must be valid for writing SIZE bytes. No alignment is required.
    unsafe fn encode_unchecked(self, dst: *mut u8);

    #[inline(always)]
    fn decode(src: &[u8]) -> Result<Self, ()> {
        if src.len() != Self::SIZE {
            return Err(());
        }
        // SAFETY: The complete wire payload is present and initialized.
        unsafe { Self::decode_unchecked(src.as_ptr()) }
    }

    /// Reads SIZE bytes without checking the input length, while validating field values.
    ///
    /// # Safety
    /// `src` must be valid for reading SIZE initialized bytes. No alignment is required.
    /// Implementations must reject invalid enum discriminants rather than construct them.
    unsafe fn decode_unchecked(src: *const u8) -> Result<Self, ()>;
}

pub trait Header: Payload {}

macro_rules! integer_payload {
    ($($ty:ty),* $(,)?) => {$(
        impl Payload for $ty {
            const SIZE: usize = core::mem::size_of::<Self>();
            #[inline(always)]
            unsafe fn encode_unchecked(self, dst: *mut u8) {
                // SAFETY: The caller supplies space for this integer; unaligned writes are supported.
                unsafe { dst.cast::<Self>().write_unaligned(self.to_le()) };
            }
            #[inline(always)]
            unsafe fn decode_unchecked(src: *const u8) -> Result<Self, ()> {
                // SAFETY: Every bit pattern is valid for an integer; the caller supplies SIZE bytes.
                Ok(Self::from_le(unsafe { src.cast::<Self>().read_unaligned() }))
            }
        }
    )*};
}

// Keep wire sizes independent of the target architecture: no usize/isize.
integer_payload!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

impl<const N: usize> Payload for [u8; N] {
    const SIZE: usize = N;
    #[inline(always)]
    unsafe fn encode_unchecked(self, dst: *mut u8) {
        // SAFETY: The caller supplies N writable bytes, separate from this owned array.
        unsafe { core::ptr::copy_nonoverlapping(self.as_ptr(), dst, N) };
    }
    #[inline(always)]
    unsafe fn decode_unchecked(src: *const u8) -> Result<Self, ()> {
        // SAFETY: Every byte pattern is valid; the caller supplies N initialized bytes.
        Ok(unsafe { src.cast::<Self>().read_unaligned() })
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
