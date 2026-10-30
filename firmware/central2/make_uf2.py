"""Package a linked nice!nano no-SoftDevice binary as nRF52840 UF2.

Format reference: Adafruit_nRF52_Bootloader 0.6.0/src/usb/uf2/uf2.h.
This creates an application update; it does not install or update a bootloader.
"""

import argparse
from pathlib import Path
import struct

APP_START = 0x1000
APP_END = 0xED000
RAM_START = 0x20000008
RAM_END = 0x20040000
FAMILY_ID = 0xADA52840
PAYLOAD_SIZE = 256


def make_uf2(binary: bytes) -> bytes:
    if len(binary) < 8:
        raise ValueError("binary has no vector table")
    if len(binary) > APP_END - APP_START:
        raise ValueError("binary overlaps reserved app-data or bootloader flash")

    initial_sp, reset = struct.unpack_from("<II", binary)
    if not RAM_START < initial_sp <= RAM_END or initial_sp % 8:
        raise ValueError("initial stack pointer is not aligned nRF52840 application RAM")
    if not reset & 1 or not APP_START <= reset & ~1 < APP_START + len(binary):
        raise ValueError("reset vector is not Thumb code inside the linked application")

    blocks = (len(binary) + PAYLOAD_SIZE - 1) // PAYLOAD_SIZE
    output = bytearray()
    for number in range(blocks):
        offset = number * PAYLOAD_SIZE
        payload = binary[offset : offset + PAYLOAD_SIZE].ljust(PAYLOAD_SIZE, b"\xff")
        header = struct.pack(
            "<8I",
            0x0A324655,
            0x9E5D5157,
            0x2000,  # Family ID present; normal application flash block.
            APP_START + offset,
            PAYLOAD_SIZE,
            number,
            blocks,
            FAMILY_ID,
        )
        output.extend(header + payload + bytes(220) + struct.pack("<I", 0x0AB16F30))
    return bytes(output)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    binary = args.binary.read_bytes()
    try:
        uf2 = make_uf2(binary)
    except ValueError as error:
        parser.error(str(error))
    args.output.write_bytes(uf2)
    print(f"UF2: {args.output} ({len(binary)} app bytes, {len(uf2) // 512} blocks)")


if __name__ == "__main__":
    main()
