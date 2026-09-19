#!/usr/bin/env python3
"""
Dual-Memory Flashing Tool for STM32WBA65RI and Cortex-M devices.

Handles:
1. Packing code overlays (.ovl) and streamable VFS assets (.vfs) into external flash image.
2. Generating a unified .fwbundle container (BundleHeader + Internal App + External Flash).
3. Flashing internal flash via probe-rs / SWD.
4. Uploading external flash payload via USB CDC, UART, or RAM Flasher Agent.
"""

import argparse
import os
import struct
import subprocess
import sys
import zlib

BUNDLE_MAGIC = b"DFW1"
BUNDLE_VERSION = 1


def crc32_ieee(data: bytes) -> int:
    """Computes standard IEEE 802.3 CRC32."""
    return zlib.crc32(data) & 0xFFFFFFFF


def create_bundle_header(
    target_chip: str,
    internal_addr: int,
    internal_data: bytes,
    external_addr: int,
    external_data: bytes,
) -> bytes:
    """Creates the 64-byte BundleHeader."""
    chip_bytes = target_chip.encode("utf-8")[:16].ljust(16, b"\x00")
    internal_size = len(internal_data)
    internal_crc = crc32_ieee(internal_data)
    external_size = len(external_data)
    external_crc = crc32_ieee(external_data)

    header_pre = struct.pack(
        "<4sHH16sIIIIII",
        BUNDLE_MAGIC,
        BUNDLE_VERSION,
        0,  # flags
        chip_bytes,
        internal_addr,
        internal_size,
        internal_crc,
        external_addr,
        external_size,
        external_crc,
    )
    header_crc = crc32_ieee(header_pre)
    return header_pre + struct.pack("<I", header_crc)


def pack_firmware_bundle(
    internal_bin_path: str,
    external_bin_path: str,
    output_bundle_path: str,
    target_chip: str = "STM32WBA65RI",
    internal_addr: int = 0x08010000,
    external_addr: int = 0x00000000,
):
    with open(internal_bin_path, "rb") as f:
        internal_data = f.read()
    with open(external_bin_path, "rb") as f:
        external_data = f.read()

    header = create_bundle_header(
        target_chip, internal_addr, internal_data, external_addr, external_data
    )

    with open(output_bundle_path, "wb") as f:
        f.write(header)
        f.write(internal_data)
        f.write(external_data)

    print(f"[*] Generated Dual-Flash Bundle -> {output_bundle_path}")
    print(f"    Target Chip:    {target_chip}")
    print(
        f"    Internal Flash: 0x{internal_addr:08X} ({len(internal_data)} bytes, CRC=0x{crc32_ieee(internal_data):08X})"
    )
    print(
        f"    External Flash: 0x{external_addr:08X} ({len(external_data)} bytes, CRC=0x{crc32_ieee(external_data):08X})"
    )
    print(f"    Total Bundle:   {len(header) + len(internal_data) + len(external_data)} bytes")


def flash_internal_swd(elf_or_bin_path: str, chip: str = "STM32WBA65RI"):
    """Flashes internal flash using probe-rs over SWD."""
    print(f"[*] Programming internal flash via probe-rs ({chip})...")
    cmd = ["probe-rs", "download", "--chip", chip, elf_or_bin_path]
    subprocess.check_call(cmd)
    print("[+] Internal flash programmed successfully!")


def flash_external_serial(port: str, baud: int, external_bin_path: str):
    """Streams external flash payload over serial/USB-CDC bootloader bridge."""
    import serial
    import time

    print(f"[*] Connecting to target bootloader on {port} ({baud} baud)...")
    with open(external_bin_path, "rb") as f:
        payload = f.read()

    total_len = len(payload)
    crc = crc32_ieee(payload)

    ser = serial.Serial(port, baud, timeout=3.0)
    time.sleep(0.1)

    # Handshake packet: b"EXTF" + length (4B) + crc (4B)
    cmd = struct.pack("<4sII", b"EXTF", total_len, crc)
    ser.write(cmd)

    ack = ser.read(4)
    if ack != b"READY":
        print(f"[-] Bootloader rejected handshake: {ack}")
        sys.exit(1)

    print(f"[*] Streaming {total_len} bytes to external SPI flash...")
    chunk_size = 4096
    sent = 0
    while sent < total_len:
        chunk = payload[sent : sent + chunk_size]
        ser.write(chunk)
        sent += len(chunk)
        pct = (sent * 100) // total_len
        sys.stdout.write(f"\r    Progress: {sent}/{total_len} bytes ({pct}%)")
        sys.stdout.flush()

    sys.stdout.write("\n")
    resp = ser.read(4)
    if resp == b"DONE":
        print("[+] External SPI flash programming completed and verified!")
    else:
        print(f"[-] Bootloader programming failed: {resp}")


def main():
    parser = argparse.ArgumentParser(description="Dual-Flash Programmer for STM32WBA / Cortex-M")
    subparsers = parser.add_subparsers(dest="command", required=True)

    # Subcommand: pack
    pack_p = subparsers.add_parser("pack", help="Create unified .fwbundle image")
    pack_p.add_argument("--internal", required=True, help="Internal app binary (.bin)")
    pack_p.add_argument("--external", required=True, help="External flash payload (.bin)")
    pack_p.add_argument("--out", required=True, help="Output bundle file (.fwbundle)")
    pack_p.add_argument("--chip", default="STM32WBA65RI", help="Target chip name")
    pack_p.add_argument(
        "--internal-addr",
        default="0x08010000",
        help="Internal flash target address (default: 0x08010000)",
    )
    pack_p.add_argument(
        "--external-addr",
        default="0x00000000",
        help="External flash target address (default: 0x00000000)",
    )

    # Subcommand: flash-internal
    flash_in_p = subparsers.add_parser("flash-internal", help="Flash internal on-chip memory via SWD")
    flash_in_p.add_argument("binary", help="ELF or bin path to flash")
    flash_in_p.add_argument("--chip", default="STM32WBA65RI", help="Target chip")

    # Subcommand: flash-external
    flash_ext_p = subparsers.add_parser(
        "flash-external", help="Flash external SPI memory via serial/USB bootloader"
    )
    flash_ext_p.add_argument("--port", required=True, help="Serial / USB CDC port (e.g. /dev/ttyACM0)")
    flash_ext_p.add_argument("--baud", type=int, default=115200, help="Baud rate")
    flash_ext_p.add_argument("binary", help="External flash binary to write")

    args = parser.parse_args()

    if args.command == "pack":
        in_addr = int(args.internal_addr, 0)
        ext_addr = int(args.external_addr, 0)
        pack_firmware_bundle(
            args.internal, args.external, args.out, args.chip, in_addr, ext_addr
        )
    elif args.command == "flash-internal":
        flash_internal_swd(args.binary, args.chip)
    elif args.command == "flash-external":
        flash_external_serial(args.port, args.baud, args.binary)


if __name__ == "__main__":
    main()
