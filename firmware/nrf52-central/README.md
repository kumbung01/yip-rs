# nRF52 central

nRF52840 standalone SWD firmware, with FLASH at 0x0 and no bootloader.
For the nice!nano UF2 bootloader layout, use [central2](../central2/README.md).

`src/main.rs` starts automatically with two links. Button messages use session 0;
received packets are logged through defmt RTT.

- RADIO, TIMER2, RTC2, EGU2/SWI2.
- PPI channels 0..7 and group 0.
- DK buttons: P0.11, P0.12, P0.24, P0.25, active-low with pull-ups.
- External HF/LF crystals.

From the repository root:

```powershell
cargo build --manifest-path firmware/nrf52-central/Cargo.toml --release --target thumbv7em-none-eabihf
```

Or run `cargo run --release` from this directory to use the configured nRF52840
SWD probe runner. Configure the correct physical probe before uploading.
The package owns its linker memory map and does not change the UF2 packages.

The matching peripheral is [nrf52-peripheral](../nrf52-peripheral/README.md).
Build/link validation does not verify radio behavior on a board.
