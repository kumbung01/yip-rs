# Independent firmware packages

`central` and `peripheral` are separate Cargo workspaces that depend on the
root `yip` library. Each selects its own chip features in `Cargo.toml` and
its build target and runner in `.cargo/config.toml`.

`central` selects nRF54LM20A Secure mode; `peripheral` selects nRF52840.
The application code was copied from `examples/test.rs`; central uses two
links, peripheral uses one. The nRF54 hardware backend and central board
initialization still need porting, so central does not build or run yet.
Radio behavior has not been verified on hardware.

## Build and run

Run `probe-rs list` to find each probe's `VID:PID:SERIAL` identifier.

Terminal 1, starting at the repository root:

```powershell
cd firmware/central
$env:PROBE_RS_PROBE = "VID:PID:CENTRAL_SERIAL"
cargo run # Requires the nRF54 backend and board initialization port first.
```

Terminal 2, starting at the repository root:

```powershell
cd firmware/peripheral
$env:PROBE_RS_PROBE = "VID:PID:PERIPHERAL_SERIAL"
cargo run
```

Run Cargo from each firmware directory. Using only `--manifest-path` from
the repository root does not load that firmware's `.cargo/config.toml`.

For separate rust-analyzer configurations, open each firmware directory as
the project root in a separate Zed window. The local `.zed/settings.json`
does not override the target or dependency features.

Each firmware has its own lockfile and target directory. Existing root
examples and the root editor configuration are unchanged.

## nRF54LM20 central configuration

Central already selects the `yip/nrf54lm20` and
`embassy-nrf/nrf54lm20-app-s` features, the
`thumbv8m.main-none-eabihf` target, and the `nRF54LM20A` probe-rs chip.
Its build script uses its own `memory.x` for a standalone Secure image
without a bootloader. Peripheral still uses the root nRF52840 memory map.

Before central can run, implement `src/hardware/nrf54` and port its
`src/main.rs` peripheral allocation, clocks, interrupt binding, and DK pins.
