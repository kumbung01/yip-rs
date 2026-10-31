pub(crate) mod clock;
pub(crate) mod device;
pub(crate) mod egu;
pub(crate) mod ppi;
pub(crate) mod radio;
mod rtc_ratio;

pub(crate) use egu::impl_yip_egu_handlers;
