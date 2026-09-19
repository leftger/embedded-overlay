//! Dual-Memory Bootloader for STM32WBA65RI.
//!
//! Manages on-chip internal flash application partitions and external SPI flash payloads.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use defmt_rtt as _;
use panic_probe as _;

/// Physical flash base address where the application binary resides.
/// (Bank 1 Sector 8, leaving the first 64 KB for this bootloader).
pub const APP_START_ADDR: usize = 0x0801_0000;

/// SRAM range for STM32WBA65RI (512 KB total).
pub const RAM_START: u32 = 0x2000_0000;
pub const RAM_END: u32 = 0x2008_0000;

/// Minimum flash address for application reset handler.
pub const APP_FLASH_MIN: u32 = 0x0801_0000;
pub const APP_FLASH_MAX: u32 = 0x0820_0000;

/// Checks if the vector table at `app_addr` contains a valid stack pointer and reset handler.
fn is_app_valid(app_addr: usize) -> bool {
    unsafe {
        let initial_sp = core::ptr::read_volatile(app_addr as *const u32);
        let reset_handler = core::ptr::read_volatile((app_addr + 4) as *const u32);

        // 1. Initial stack pointer must point inside physical SRAM
        if initial_sp < RAM_START || initial_sp > RAM_END {
            return false;
        }

        // 2. Reset handler must point to valid flash memory and have the Thumb bit (LSB=1) set
        if reset_handler < APP_FLASH_MIN || reset_handler > APP_FLASH_MAX {
            return false;
        }
        if reset_handler & 1 == 0 {
            return false;
        }

        true
    }
}

/// Jumps execution to the application at `app_addr`.
///
/// # Safety
/// Disables interrupts, configures VTOR, restores MSP, and branches to the reset handler.
#[inline(never)]
unsafe fn jump_to_app(app_addr: usize) -> ! {
    defmt::info!("Relocating VTOR to 0x{:08x} and jumping to application...", app_addr);

    // 1. Disable all interrupts during vector table switchover
    cortex_m::interrupt::disable();

    // 2. Read initial SP and Reset Handler
    let initial_sp = core::ptr::read_volatile(app_addr as *const u32);
    let reset_handler_addr = core::ptr::read_volatile((app_addr + 4) as *const u32);

    // 3. Relocate Cortex-M Vector Table Offset Register (VTOR)
    let p = cortex_m::Peripherals::steal();
    p.SCB.vtor.write(app_addr as u32);

    // 4. Memory barrier
    cortex_m::asm::dsb();
    cortex_m::asm::isb();

    // 5. Bootstrap: atomically sets MSP and branches to reset handler
    cortex_m::asm::bootstrap(initial_sp as *const u32, reset_handler_addr as *const u32);
}

#[entry]
fn main() -> ! {
    defmt::info!("=== STM32WBA Dual-Memory Bootloader ===");

    // Check if the application partition contains a valid vector table
    if is_app_valid(APP_START_ADDR) {
        defmt::info!("Valid application detected at 0x{:08x}", APP_START_ADDR);
        unsafe { jump_to_app(APP_START_ADDR) };
    } else {
        defmt::warn!("No valid application found at 0x{:08x}! Entering DFU flasher mode...", APP_START_ADDR);
    }

    // In DFU mode, the bootloader listens on transport for incoming DFW1 bundle
    loop {
        cortex_m::asm::wfi();
    }
}
