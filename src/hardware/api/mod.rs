//! Hardware contracts used by the role state machines.
//! No default methods: every backend must explicitly implement the API.
//!
//! Add role-facing methods here, then implement them in both `nrf52` and `nrf54`.
//! Constructors and register-level helpers that differ by chip stay in the backends.
//! Calls use concrete drivers, so these traits do not introduce dynamic dispatch.
//!
//! rust-analyzer checks the backend enabled by the selected Cargo features.
//! Check both configurations when changing these contracts:
//! ```text
//! cargo check --lib --features defmt --target thumbv7em-none-eabihf
//! cargo check --lib --no-default-features --features nrf54lm20,defmt --target thumbv8m.main-none-eabihf
//! ```

pub(crate) mod clock;
pub(crate) mod egu;
pub(crate) mod ppi;
pub(crate) mod radio;

pub(crate) use clock::{ClockDriver, ClockTimestamp};
pub(crate) use egu::EguDriver;
pub(crate) use ppi::PpiDriver;
pub(crate) use radio::RadioDriver;
