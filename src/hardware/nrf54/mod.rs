//! nRF54LM20 backend. Hardware support has not been ported yet.

pub(crate) mod clock;
pub mod cyccnt;
pub(crate) mod device;
pub(crate) mod egu;
pub(crate) mod ppi;
pub(crate) mod radio;

pub(crate) use egu::impl_yip_egu_handlers;
