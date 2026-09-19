//! Build script utilities for automated linker script generation and memory region layout.

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
use std::format;
#[cfg(feature = "std")]
use std::fs::File;
#[cfg(feature = "std")]
use std::io::{self, Write};
#[cfg(feature = "std")]
use std::path::Path;
#[cfg(feature = "std")]
use std::println;
#[cfg(feature = "std")]
use std::string::String;

/// Configuration for automated linker script generation in `build.rs`.
#[derive(Debug, Clone)]
pub struct OverlayLinkerConfig {
    /// Internal on-chip flash memory base address (e.g. `0x0801_0000`).
    pub flash_origin: u32,
    /// Internal flash memory size in bytes (e.g. `512 * 1024`).
    pub flash_length: u32,
    /// Physical SRAM base address (e.g. `0x2000_0000`).
    pub ram_origin: u32,
    /// General application RAM length in bytes (excluding overlay slots).
    pub ram_length: u32,
    /// Dedicated RAM Overlay Slot A base address (e.g. `0x2006_0000`).
    pub slot_a_origin: u32,
    /// RAM Overlay Slot A length in bytes (e.g. `32 * 1024`).
    pub slot_a_length: u32,
    /// Dedicated RAM Overlay Slot B base address (e.g. `0x2006_8000`).
    pub slot_b_origin: u32,
    /// RAM Overlay Slot B length in bytes (e.g. `32 * 1024`).
    pub slot_b_length: u32,
    /// External Flash / SD Card base address or offset.
    pub ext_flash_origin: u32,
    /// External Flash length in bytes (e.g. `16 * 1024 * 1024`).
    pub ext_flash_length: u32,
}

impl Default for OverlayLinkerConfig {
    fn default() -> Self {
        Self {
            flash_origin: 0x0801_0000,
            flash_length: 512 * 1024,
            ram_origin: 0x2000_0000,
            ram_length: 384 * 1024,
            slot_a_origin: 0x2006_0000,
            slot_a_length: 32 * 1024,
            slot_b_origin: 0x2006_8000,
            slot_b_length: 32 * 1024,
            ext_flash_origin: 0x9000_0000,
            ext_flash_length: 16 * 1024 * 1024,
        }
    }
}

impl OverlayLinkerConfig {
    /// Generates a GNU LD / LLD linker script with VMA / LMA overlay sections.
    #[cfg(feature = "std")]
    pub fn generate_script_content(&self) -> String {
        format!(
            r#"/* Auto-generated Linker Script for RAM Code Overlays by embedded-overlay */

MEMORY
{{
  FLASH         (rx)  : ORIGIN = 0x{flash_origin:08X}, LENGTH = {flash_length}
  RAM           (rwx) : ORIGIN = 0x{ram_origin:08X}, LENGTH = {ram_length}
  RAM_OVERLAY_A (rwx) : ORIGIN = 0x{slot_a_origin:08X}, LENGTH = {slot_a_length}
  RAM_OVERLAY_B (rwx) : ORIGIN = 0x{slot_b_origin:08X}, LENGTH = {slot_b_length}
  EXT_FLASH     (r)   : ORIGIN = 0x{ext_flash_origin:08X}, LENGTH = {ext_flash_length}
}}

SECTIONS
{{
  /* Overlay Slot A execution region (VMA) */
  .ram_overlay_a (NOLOAD) : ALIGN(16)
  {{
    __sram_overlay_a_start = .;
    . += LENGTH(RAM_OVERLAY_A);
    __sram_overlay_a_end = .;
  }} > RAM_OVERLAY_A

  /* Overlay Slot B execution region (VMA) */
  .ram_overlay_b (NOLOAD) : ALIGN(16)
  {{
    __sram_overlay_b_start = .;
    . += LENGTH(RAM_OVERLAY_B);
    __sram_overlay_b_end = .;
  }} > RAM_OVERLAY_B

  /* Overlay Code Sections placed in external storage (LMA) */
  .overlay_sections : ALIGN(4)
  {{
    __overlay_code_start = .;
    *(.overlay.*)
    __overlay_code_end = .;
  }} > EXT_FLASH
}}
"#,
            flash_origin = self.flash_origin,
            flash_length = self.flash_length,
            ram_origin = self.ram_origin,
            ram_length = self.ram_length,
            slot_a_origin = self.slot_a_origin,
            slot_a_length = self.slot_a_length,
            slot_b_origin = self.slot_b_origin,
            slot_b_length = self.slot_b_length,
            ext_flash_origin = self.ext_flash_origin,
            ext_flash_length = self.ext_flash_length,
        )
    }

    /// Writes the generated linker script to a file and configures cargo to link it.
    #[cfg(feature = "std")]
    pub fn emit_to_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let content = self.generate_script_content();
        let mut file = File::create(path.as_ref())?;
        file.write_all(content.as_bytes())?;

        if let Some(parent) = path.as_ref().parent() {
            println!("cargo:rustc-link-search={}", parent.display());
        }
        if let Some(file_name) = path.as_ref().file_name().and_then(|n| n.to_str()) {
            println!("cargo:rustc-link-arg=-T{}", file_name);
        }
        Ok(())
    }
}
