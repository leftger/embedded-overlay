//! Host CLI tool for packaging code overlays and streamable VFS asset blobs.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use embedded_overlay::crc::crc32;
use embedded_overlay::header::{OverlayHeader, VfsAssetEntry, VfsSuperblock};

fn print_usage(prog: &str) {
    eprintln!("Usage:");
    eprintln!("  {prog} overlay <input.bin> <module_id_hex_or_dec> <output.ovl>");
    eprintln!("  {prog} vfs <asset_dir> <output.vfs>");
    eprintln!("  {prog} bundle <internal.bin> <external.bin> <target_chip> <internal_addr> <external_addr> <output.fwbundle>");
}

fn pack_overlay(input_path: &str, module_id_str: &str, output_path: &str) -> io::Result<()> {
    let module_id = if module_id_str.starts_with("0x") || module_id_str.starts_with("0X") {
        u32::from_str_radix(&module_id_str[2..], 16)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
    } else {
        module_id_str
            .parse::<u32>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
    };

    let payload = fs::read(input_path)?;
    let payload_crc = crc32(&payload);

    let header = OverlayHeader::new(
        module_id,
        payload.len() as u32,
        0, // entry point at offset 0
        0, // relocatable / fixed slot
        payload_crc,
    );

    let mut out_file = File::create(output_path)?;
    out_file.write_all(&header.to_bytes())?;
    out_file.write_all(&payload)?;

    println!(
        "Packed overlay: module_id=0x{module_id:08X}, size={} bytes, crc=0x{payload_crc:08X} -> {output_path}",
        payload.len()
    );
    Ok(())
}

fn pack_vfs(asset_dir: &str, output_path: &str) -> io::Result<()> {
    let dir = Path::new(asset_dir);
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{asset_dir} is not a directory"),
        ));
    }

    let mut entries = Vec::new();
    let mut data_payload = Vec::new();
    let mut asset_id = 1u32;

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let mut file = File::open(&path)?;
            let mut content = Vec::new();
            file.read_to_end(&mut content)?;

            let offset = data_payload.len() as u32;
            let len = content.len() as u32;

            entries.push(VfsAssetEntry {
                asset_id,
                flash_offset: offset,
                length: len,
                flags: 0,
                crc16: 0,
            });

            data_payload.extend_from_slice(&content);
            // Align next asset to 4 bytes for clean DMA transfers
            while data_payload.len() % 4 != 0 {
                data_payload.push(0);
            }

            println!(
                "  Added asset #{asset_id}: {} ({} bytes at offset 0x{offset:06X})",
                path.display(),
                len
            );
            asset_id += 1;
        }
    }

    let index_offset = VfsSuperblock::SIZE as u32;
    let data_offset = index_offset + (entries.len() * VfsAssetEntry::SIZE) as u32;
    let total_bytes = data_offset + data_payload.len() as u32;

    let superblock =
        VfsSuperblock::new(entries.len() as u16, index_offset, data_offset, total_bytes);

    let mut out_file = File::create(output_path)?;
    out_file.write_all(&superblock.to_bytes())?;
    for e in &entries {
        out_file.write_all(&e.to_bytes())?;
    }
    out_file.write_all(&data_payload)?;

    println!(
        "Packed VFS: {} assets, total {} bytes -> {output_path}",
        entries.len(),
        total_bytes
    );
    Ok(())
}

fn parse_hex_or_dec(s: &str) -> io::Result<u32> {
    if s.starts_with("0x") || s.starts_with("0X") {
        u32::from_str_radix(&s[2..], 16)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    } else {
        s.parse::<u32>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }
}

fn pack_bundle(
    internal_path: &str,
    external_path: &str,
    target_chip: &str,
    internal_addr_str: &str,
    external_addr_str: &str,
    output_path: &str,
) -> io::Result<()> {
    use embedded_overlay::bundle::BundleHeader;

    let internal_addr = parse_hex_or_dec(internal_addr_str)?;
    let external_addr = parse_hex_or_dec(external_addr_str)?;

    let internal_data = fs::read(internal_path)?;
    let external_data = fs::read(external_path)?;

    let internal_crc = crc32(&internal_data);
    let external_crc = crc32(&external_data);

    let header = BundleHeader::new(
        target_chip,
        internal_addr,
        internal_data.len() as u32,
        internal_crc,
        external_addr,
        external_data.len() as u32,
        external_crc,
    );

    let mut out_file = File::create(output_path)?;
    out_file.write_all(&header.to_bytes())?;
    out_file.write_all(&internal_data)?;
    out_file.write_all(&external_data)?;

    println!("Packed Dual-Flash Firmware Bundle -> {output_path}");
    println!("  Target Chip:    {target_chip}");
    println!(
        "  Internal Flash: 0x{internal_addr:08X} ({} bytes, CRC=0x{internal_crc:08X})",
        internal_data.len()
    );
    println!(
        "  External Flash: 0x{external_addr:08X} ({} bytes, CRC=0x{external_crc:08X})",
        external_data.len()
    );
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage(&args[0]);
        return Ok(());
    }

    match args[1].as_str() {
        "overlay" => {
            if args.len() != 5 {
                eprintln!("Usage: {} overlay <input.bin> <module_id> <output.ovl>", args[0]);
                std::process::exit(1);
            }
            pack_overlay(&args[2], &args[3], &args[4])
        }
        "vfs" => {
            if args.len() != 4 {
                eprintln!("Usage: {} vfs <asset_dir> <output.vfs>", args[0]);
                std::process::exit(1);
            }
            pack_vfs(&args[2], &args[3])
        }
        "bundle" => {
            if args.len() != 8 {
                eprintln!(
                    "Usage: {} bundle <internal.bin> <external.bin> <chip> <internal_addr> <external_addr> <output.fwbundle>",
                    args[0]
                );
                std::process::exit(1);
            }
            pack_bundle(&args[2], &args[3], &args[4], &args[5], &args[6], &args[7])
        }
        _ => {
            print_usage(&args[0]);
            std::process::exit(1);
        }
    }
}
