//! nRF54LM20 central/peripheral backend using GRTC, TIMER10 and DPPI/PPIB.

pub(crate) mod clock;
pub(crate) mod device;
pub(crate) mod egu;
pub(crate) mod ppi;
pub(crate) mod radio;

pub(crate) use egu::impl_yip_egu_handlers;
