# nRF54 peripheral

nRF54LM20A application core, Secure mode, without a bootloader.

`src/main.rs` uses the same hardware resources and DK button pins as
[nrf54-central](../nrf54-central/README.md). Embassy's time driver uses GRTC_CH11;
the radio scheduler owns GRTC_CH0, independently of the three-second send timer.

ACK automatic READY -> START is disabled: after
preparing the ACK, the role arms a one-shot TXREADY -> START route with an
already-ready fallback. It does not busy-wait for TX ramp-up.

- Initially waits for a button before calling `yip::peripheral::start()`.
- The most recent button received while scanning is sent after connection.
- Connected presses send a 200-byte message; incoming packets are logged.
- While connected, sends 1, 2, 3, 4 repeatedly at three-second intervals.
- Each received message prints cumulative send/receive statistics.
- There is no inactivity shutdown.

`PERIPHERAL_ID` is 1 for the second peripheral on the two-link central.
Change `SEND_INTERVAL` to adjust the periodic send interval.

The YIP dependency enables `reduce-rram-latency`: RRAM stays in Standby from
connected RX scheduling until RX completion, then returns to PowerOff. Stop
also restores PowerOff. Remove the feature from `Cargo.toml` to leave RRAMC
power settings unchanged. This option works with or without `defmt`.

From the repository root:

```powershell
cargo build --manifest-path firmware/nrf54-peripheral/Cargo.toml --release --target thumbv8m.main-none-eabihf
```

Or run `cargo run --release` from this directory with the appropriate nRF54
probe runner. Logs use defmt RTT, not USB serial. The local memory map reserves
the KMU SRAM area and is independent of the nRF52 and UF2 firmware layouts.

Firmware build/link checks do not verify physical radio or ACK timing.
