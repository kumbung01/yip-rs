//! Chip-specific hardware selected by the application's Cargo feature.

pub(crate) mod api;
mod context;

#[cfg(all(feature = "nrf52840", feature = "nrf54lm20"))]
compile_error!("Select only one chip feature: nrf52840 or nrf54lm20.");

#[cfg(not(any(feature = "nrf52840", feature = "nrf54lm20")))]
compile_error!("Select a chip feature: nrf52840 or nrf54lm20.");

#[cfg(feature = "nrf52840")]
mod nrf52;
#[cfg(feature = "nrf52840")]
pub(crate) use nrf52::*;

#[cfg(feature = "nrf54lm20")]
mod nrf54;
#[cfg(feature = "nrf54lm20")]
pub(crate) use nrf54::*;

#[cfg(feature = "nrf52840")]
pub(crate) use nrf52::egu::Nrf52Egu as Egu;

#[cfg(feature = "nrf54lm20")]
pub(crate) use nrf54::egu::Nrf54Egu as Egu;
