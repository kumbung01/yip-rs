# nRF54 peripheral

nRF54LM20A application core, Secure mode, without a bootloader.

`src/main.rs` uses the same hardware resources and DK button pins as
[nrf54-central](../nrf54-central/README.md). Embassy's time driver uses GRTC_CH11;
the radio scheduler owns GRTC_CH0, so the ten-second timer remains operational
while the radio role is stopped.

ACK automatic READY -> START is disabled: after
preparing the ACK, the role arms a one-shot TXREADY -> START route with an
already-ready fallback. It does not busy-wait for TX ramp-up.

- Initially waits for a button before calling `yip::peripheral::start()`.
- Further presses during scanning or connection extend the deadline by ten seconds.
- The most recent button received while scanning is sent after connection.
- Connected presses send a 200-byte message; incoming packets are logged.
- Ten seconds without a button stops the peripheral and waits for another press.

`PERIPHERAL_ID` defaults to 0. Set it to 1 for a second peripheral on the
two-link central. Change `IDLE_TIMEOUT` to adjust the button inactivity timeout.

From the repository root:

```powershell
cargo build --manifest-path firmware/nrf54-peripheral/Cargo.toml --release --target thumbv8m.main-none-eabihf
```

Or run `cargo run --release` from this directory with the appropriate nRF54
probe runner. Logs use defmt RTT, not USB serial. The local memory map reserves
the KMU SRAM area and is independent of the nRF52 and UF2 firmware layouts.

Firmware build/link checks do not verify physical radio or ACK timing.
