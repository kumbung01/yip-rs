# Independent firmware packages

Explicit nRF52 role entry points are available in
[`nrf52-central/src/main.rs`](nrf52-central/src/main.rs) and
[`nrf52-peripheral/src/main.rs`](nrf52-peripheral/src/main.rs). Both are nRF52840
standalone SWD images with FLASH at 0x0; `central2` remains the nice!nano UF2 image.

Explicit nRF54 role entry points are available in
[`nrf54-central/src/main.rs`](nrf54-central/src/main.rs) and
[`nrf54-peripheral/src/main.rs`](nrf54-peripheral/src/main.rs), each with its own
Cargo workspace, linker memory map and target configuration. See their READMEs
for build commands, button pins and start/stop behavior.

`central`, `central2`, and `peripheral` are separate Cargo workspaces that depend on the
root `yip` library. Each selects its own chip features in `Cargo.toml` and
its build target and runner in `.cargo/config.toml`.

`central` selects nRF54LM20A Secure mode; `peripheral` selects nRF52840.
`central2` is an nRF52840 nice!nano central with a UF2 application layout
for the reported bootloader without SoftDevice. See [central2/README.md](central2/README.md).
The application code was copied from `examples/test.rs`; central uses two
links, peripheral uses one. The nRF54 hardware backend and central board
initialization now compile; runtime behavior still requires board verification.
Radio behavior has not been verified on hardware.

## Build and run

Run `probe-rs list` to find each probe's `VID:PID:SERIAL` identifier.

Terminal 1, starting at the repository root:

```powershell
cd firmware/central
$env:PROBE_RS_PROBE = "VID:PID:CENTRAL_SERIAL"
cargo run
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

Both nRF54 role paths are implemented and compile. Radio operation still requires
hardware verification. See [the nRF54 backend notes](../src/hardware/nrf54/README.md)
for the five RADIO channels, two groups, timing tests and peripheral example.

## nice!nano central UF2

From the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File firmware/central2/build-uf2.ps1
```

Copy `central2/target/thumbv7em-none-eabihf/release/yip-central2.uf2` to the
`NICENANO` bootloader drive. This package links at 0x1000 and is only for
`SoftDevice: not found`. Its build script does not flash a board or update
the bootloader.
