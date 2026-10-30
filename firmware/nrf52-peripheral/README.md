# nRF52 peripheral

nRF52840 standalone SWD firmware, with FLASH at 0x0 and no bootloader.

- RADIO, TIMER2, RTC2, EGU2/SWI2, PPI channels 0..7 and group 0.
- Embassy timers use RTC1; Yip uses RTC2.
- DK buttons: P0.11, P0.12, P0.24, P0.25, active-low with pull-ups.
- External HF/LF crystals; defmt RTT logs.

The first button starts the peripheral. Presses while scanning or connected
extend the inactivity deadline by ten seconds. The most recent scanning-time
button is sent after connection; connected presses send 200-byte messages.
Ten seconds without a press stops the radio role and waits for another button.

`PERIPHERAL_ID` defaults to 0; use 1 for the second peripheral on a two-link
central. `IDLE_TIMEOUT` controls the ten-second deadline.

From the repository root:

```powershell
cargo build --manifest-path firmware/nrf52-peripheral/Cargo.toml --release --target thumbv7em-none-eabihf
```

Or run `cargo run --release` from this directory with the correct nRF52840 SWD
probe runner. This standalone layout is not a UF2 bootloader application.
Build/link validation does not verify radio behavior on a board.

With `defmt`, `ACK timing` logs compare CRCOK-to-IRQ latency, IRQ-to-receive-handler
latency, handler-to-buffer-setup time and total `event_to_ptr_us`. Values are in
microseconds from the 1 MHz TIMER2, including CPU sleep/wakeup time. Diagnostics
capture CRCOK in CC3 and reuse CC0 after its RXEN event; they work with TIMER2's
four capture channels. The RX timeout route is disabled at RADIO DISABLED, and
the timer keeps running until the pre-buffer-setup measurement. nRF52 retains
automatic ACK START. Capture overhead is included; nRF54's separate TXEN-to-buffer
measurement is not available in this four-channel configuration.

`ACK cycles` reports the same DWT phase measurements as nRF54: RADIO events,
header parsing, RX processing, TX commit/retry, payload construction, power
control, buffer/address register writes, and ACK arming. Values include snapshot
overhead. Compare both chips using the same diagnostic version.
