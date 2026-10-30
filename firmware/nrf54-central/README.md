# nRF54 central

nRF54LM20A application core, Secure mode, without a bootloader.

`src/main.rs` starts the central automatically with two links. It logs received
packets and sends 200-byte button messages through session 0.

- TIMER10, GRTC_CH0, EGU10.
- PPI10 channels 0..4, PPI20 channel 0, PPI10 groups 0 and 1.
- PPIB21_CH0 -> PPIB11_CH0.
- Buttons: P1.26, P1.09, P1.08, P0.05, active-low with pull-ups.
- External HF/LF crystals; defmt RTT logging through SWD.

From the repository root:

```powershell
cargo build --manifest-path firmware/nrf54-central/Cargo.toml --release --target thumbv8m.main-none-eabihf
```

Or run `cargo run --release` from this directory to use the configured nRF54
probe runner. Select the appropriate probe for each physical board in the
repository's `.cargo/config.toml` when running two boards.

The matching peripheral is [../nrf54-peripheral](../nrf54-peripheral/README.md).
Firmware build/link checks do not verify physical radio or ACK timing.
