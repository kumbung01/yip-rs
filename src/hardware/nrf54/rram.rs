//! RRAM active-idle policy missing from the currently pinned PAC.
use embassy_nrf::pac::{
    self,
    common::{RW, Reg},
};

const MODE_MASK: u32 = 0b11;
fn standby_config() -> Reg<u32, RW> {
    // SAFETY: nRF54LM20A/B datasheet POWER.STANDBYCONFIG is a 32-bit RW
    // register at RRAMC + 0x514, MODE in bits 1:0. The pinned PAC omits it.
    // https://docs.nordicsemi.com/r/bundle/ps_nrf54lm20a/page/rramc.html-register.power.standbyconfig
    unsafe { Reg::from_ptr(pac::RRAMC.as_ptr().cast::<u8>().add(0x514).cast()) }
}

/// Disable forced active-idle standby, retaining the other register bits.
#[cfg(feature = "reduce-rram-latency")]
pub(crate) fn set_normal() {
    standby_config().modify(|value| *value &= !MODE_MASK);
}

#[cfg(any(feature = "defmt", feature = "log"))]
pub(crate) fn mode() -> u8 {
    (standby_config().read() & MODE_MASK) as u8
}
