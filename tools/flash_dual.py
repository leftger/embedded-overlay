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
import re
import struct
import subprocess
import sys
import zlib

BUNDLE_MAGIC = b"DFW1"
BUNDLE_VERSION = 1

OVERLAY_MAGIC = b"OVL1"
OVERLAY_VERSION = 1

OVERLAY_DIR_MAGIC = b"OVLD"
OVERLAY_DIR_VERSION = 1


def crc32_ieee(data: bytes) -> int:
    """Computes standard IEEE 802.3 CRC32."""
    return zlib.crc32(data) & 0xFFFFFFFF


def fnv1a_32(s: str) -> int:
    """Computes 32-bit FNV-1a hash matching embedded-overlay::crc::fnv1a_hash."""
    h = 0x811C9DC5
    for b in s.encode("utf-8"):
        h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
    return h


def extract_overlay_sections(elf_path: str) -> list[tuple[str, str]]:
    """Discovers all .overlay.* sections in the ELF binary using readelf."""
    cmd = ["readelf", "-W", "-S", elf_path]
    output = subprocess.check_output(cmd, text=True)
    sections = []
    for line in output.splitlines():
        match = re.search(r"\[\s*\d+\]\s+(\.overlay\.([a-zA-Z0-9_]+))\s+", line)
        if match:
            full_sec = match.group(1)
            fn_name = match.group(2)
            sections.append((full_sec, fn_name))
    return sections


def dump_section_bytes(elf_path: str, section_name: str) -> bytes:
    """Dumps raw section machine code using llvm-objcopy or arm-none-eabi-objcopy."""
    import tempfile

    with tempfile.NamedTemporaryFile(delete=False) as tmp:
        tmp_name = tmp.name
    try:
        tools = ["llvm-objcopy", "arm-none-eabi-objcopy", "objcopy"]
        success = False
        for tool in tools:
            try:
                cmd = [tool, f"--dump-section={section_name}={tmp_name}", elf_path]
                res = subprocess.call(
                    cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
                )
                if (
                    res == 0
                    and os.path.exists(tmp_name)
                    and os.path.getsize(tmp_name) > 0
                ):
                    success = True
                    break
            except FileNotFoundError:
                continue
        if not success:
            raise RuntimeError(
                f"Failed to dump section {section_name} from {elf_path}"
            )
        with open(tmp_name, "rb") as f:
            return f.read()
    finally:
        if os.path.exists(tmp_name):
            os.remove(tmp_name)


def build_overlay_container(fn_name: str, payload: bytes) -> tuple[int, bytes]:
    """Wraps raw machine code into a 32-byte header + payload .ovl container."""
    module_id = fnv1a_32(fn_name)
    payload_crc = crc32_ieee(payload)
    code_size = len(payload)
    header_pre = struct.pack(
        "<4sHHIIIII",
        OVERLAY_MAGIC,
        OVERLAY_VERSION,
        0,  # flags
        module_id,
        code_size,
        0,  # entry_offset
        0,  # ram_target_addr
        payload_crc,
    )
    header_crc = crc32_ieee(header_pre)
    header = header_pre + struct.pack("<I", header_crc)
    return module_id, header + payload


def build_external_flash_image(
    overlays: list[tuple[str, int, bytes]],
) -> bytes:
    """
    Builds the complete external flash binary:
    - 32-byte OverlayDirectory header (b"OVLD")
    - Array of 16-byte OverlayDirectoryEntry records
    - Sequential .ovl containers (aligned to 4 bytes)
    """
    entry_count = len(overlays)
    index_offset = 32
    entries_size = 16 * entry_count
    payload_start_offset = index_offset + entries_size

    current_offset = payload_start_offset
    entries = []
    payloads = bytearray()

    for _fn_name, module_id, ovl_bytes in overlays:
        while (current_offset % 4) != 0:
            payloads.append(0)
            current_offset += 1

        code_size = len(ovl_bytes) - 32
        ovl_crc = crc32_ieee(ovl_bytes)
        entries.append((module_id, current_offset, code_size, ovl_crc))
        payloads.extend(ovl_bytes)
        current_offset += len(ovl_bytes)

    total_bytes = current_offset

    dir_pre = struct.pack(
        "<4sHHII",
        OVERLAY_DIR_MAGIC,
        OVERLAY_DIR_VERSION,
        entry_count,
        index_offset,
        total_bytes,
    )
    dir_crc = crc32_ieee(dir_pre[:16])
    dir_header = struct.pack(
        "<4sHHIII12s",
        OVERLAY_DIR_MAGIC,
        OVERLAY_DIR_VERSION,
        entry_count,
        index_offset,
        total_bytes,
        dir_crc,
        b"\x00" * 12,
    )

    entries_bytes = bytearray()
    for mod_id, off, sz, crc in entries:
        entries_bytes.extend(struct.pack("<IIII", mod_id, off, sz, crc))

    return dir_header + entries_bytes + payloads


def auto_run(
    elf_path: str,
    chip: str = "STM32WBA65RI",
    protocol: str = "swd",
    dry_run: bool = False,
):
    """
    Zero-touch automated runner:
    1. Inspects ELF binary and extracts all .overlay.* sections.
    2. Derives FNV-1a IDs and packs .ovl containers + OverlayDirectory into external flash image.
    3. Emits .fwbundle container.
    4. Runs probe-rs run to download and stream defmt/RTT logs.
    """
    print(f"[*] embedded-overlay Auto-Runner: Inspecting {elf_path}")
    if not os.path.isfile(elf_path):
        print(f"[-] Error: ELF file not found: {elf_path}")
        sys.exit(1)

    sections = extract_overlay_sections(elf_path)
    elf_dir = os.path.dirname(os.path.abspath(elf_path))
    ext_flash_path = os.path.join(elf_dir, "ext_flash.bin")
    bundle_path = os.path.join(elf_dir, "firmware.fwbundle")

    if not sections:
        print("[*] No .overlay.* sections detected in ELF. Standard binary detected.")
    else:
        print(f"[+] Discovered {len(sections)} code overlay(s) in ELF:")
        packed_overlays = []
        for full_sec, fn_name in sections:
            payload = dump_section_bytes(elf_path, full_sec)
            mod_id, ovl_bytes = build_overlay_container(fn_name, payload)
            packed_overlays.append((fn_name, mod_id, ovl_bytes))
            print(
                f"    - {fn_name}: ID=0x{mod_id:08X}, {len(payload)} bytes code, CRC=0x{crc32_ieee(payload):08X}"
            )

        ext_flash_data = build_external_flash_image(packed_overlays)
        with open(ext_flash_path, "wb") as f:
            f.write(ext_flash_data)
        print(
            f"[+] Generated External Flash Image -> {ext_flash_path} ({len(ext_flash_data)} bytes)"
        )

        # Convert ELF to raw internal bin for firmware bundle if needed
        tmp_internal_bin = os.path.join(elf_dir, "internal_app.bin")
        try:
            subprocess.call(
                ["llvm-objcopy", "-O", "binary", elf_path, tmp_internal_bin],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            if os.path.exists(tmp_internal_bin):
                pack_firmware_bundle(
                    internal_bin_path=tmp_internal_bin,
                    external_bin_path=ext_flash_path,
                    output_bundle_path=bundle_path,
                    target_chip=chip,
                )
                os.remove(tmp_internal_bin)
        except Exception as e:
            print(f"[-] Note: Bundle packaging skipped ({e})")

    if dry_run:
        print("[+] Dry-run completed. All overlays extracted and packaged successfully!")
        return

    # Execute target runner (probe-rs run)
    cmd = ["probe-rs", "run", "--chip", chip, "--protocol", protocol, elf_path]
    print(f"[*] Executing target: {' '.join(cmd)}")
    sys.stdout.flush()
    try:
        subprocess.check_call(cmd)
    except subprocess.CalledProcessError as e:
        sys.exit(e.returncode)
    except FileNotFoundError:
        print("[-] probe-rs not found on PATH. Install via: cargo install probe-rs-tools")
        sys.exit(1)
    except KeyboardInterrupt:
        pass


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

    # Subcommand: auto-run (Cargo runner interface)
    auto_p = subparsers.add_parser(
        "auto-run",
        help="Zero-touch automated runner for 'cargo run': extracts overlays, builds external flash image, and runs target",
    )
    auto_p.add_argument("binary", help="Target ELF binary produced by cargo")
    auto_p.add_argument("--chip", default="STM32WBA65RI", help="Target chip name")
    auto_p.add_argument(
        "--protocol", default="swd", help="Probe protocol (default: swd)"
    )
    auto_p.add_argument(
        "--dry-run", action="store_true", help="Extract and pack without flashing hardware"
    )

    args = parser.parse_args()

    if args.command == "auto-run":
        auto_run(args.binary, args.chip, args.protocol, args.dry_run)
    elif args.command == "pack":
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

