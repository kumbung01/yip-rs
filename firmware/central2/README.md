# nice!nano central (nRF52840, UF2)

Two-link central firmware for the reported `nRF52840-nicenano` bootloader:
`UF2 Bootloader 0.6.0`, **`SoftDevice: not found`**.

The application is linked at **0x1000**, immediately after the MBR. It ends
before 0xED000, preserving app-data and bootloader flash. The first 8 RAM bytes
are reserved for MBR interrupt forwarding. This package has its own `memory.x`;
the existing central and peripheral packages keep their memory layouts.

Do not use this layout if `INFO_UF2.TXT` reports an installed SoftDevice.
See the [bootloader address selection][addresses] and [UF2 block format][format].

## Build

From the repository root, with Python 3 and Rust's thumb target installed:

```powershell
rustup target add thumbv7em-none-eabihf
rustup component add llvm-tools
cargo install cargo-binutils
powershell -NoProfile -ExecutionPolicy Bypass -File firmware/central2/build-uf2.ps1
```

The build script uses the lockfile, verifies that the ELF vector table is at
0x1000, converts it to a binary, and validates the stack/reset vectors and flash
bounds before writing:

```text
firmware/central2/target/thumbv7em-none-eabihf/release/yip-central2.uf2
```

The Python converter uses only the standard library. It packages the application
with the nRF52840 family ID `0xADA52840`; it does not update the bootloader.

## Upload

1. Connect USB and double-tap reset (or briefly connect RST to GND twice).
2. Confirm that the `NICENANO` drive's `INFO_UF2.TXT` says `SoftDevice: not found`.
3. Copy `yip-central2.uf2` to that drive. The board should reset into the app.

Use the UF2 file for upload. `cargo run` can inherit the repository's SWD
probe runner and is not the USB upload procedure for this package.

## Hardware

- RADIO, TIMER2, RTC2, EGU2/SWI2, PPI channels 0..7, PPI groups 0 and 1.
- External HF/LF crystals, matching nice!nano hardware.
- Four active-low buttons to GND on P0.06, P0.08, P0.17, P0.20, with pull-ups.
  Adjust these four pin assignments in `src/main.rs` for your wiring.
- Starts the central automatically, with two sessions. Button messages use
  session 0, as in the original central firmware.
- Logging and RTT are disabled. The application does not provide USB serial;
  the UF2 USB drive belongs to the bootloader and disappears after startup.

Building and UF2 validation do not verify radio operation on the physical board.

[addresses]: https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/0.6.0/lib/sdk11/components/libraries/bootloader_dfu/dfu_types.h
[format]: https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/0.6.0/src/usb/uf2/uf2.h
