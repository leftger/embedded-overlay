//! Processor memory barriers and cache synchronization for dynamic code execution.

/// Synchronizes the instruction stream after loading new machine code into RAM.
///
/// On ARM Cortex-M targets, this executes:
/// 1. `dsb` (Data Synchronization Barrier) - Ensures all memory stores are committed.
/// 2. `isb` (Instruction Synchronization Barrier) - Flushes the processor pipeline so
///    that subsequently fetched instructions observe the new code in RAM.
///
/// On host/test platforms, this performs a compiler barrier.
#[inline(always)]
pub fn sync_instruction_memory() {
    #[cfg(all(target_arch = "arm", feature = "cortex-m"))]
    {
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
    }
    #[cfg(not(all(target_arch = "arm", feature = "cortex-m")))]
    {
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}

/// Validates that a target RAM address and entry point meet Thumb-2 alignment rules.
///
/// Cortex-M requires 2-byte or 4-byte instruction alignment.
#[inline]
pub fn validate_thumb_alignment(addr: usize, entry_offset: usize) -> bool {
    // RAM buffer should be at least 4-byte aligned
    if addr % 4 != 0 {
        return false;
    }
    // Entry point offset should be at least 2-byte aligned (halfword instruction)
    if entry_offset % 2 != 0 {
        return false;
    }
    true
}

/// Converts a function entry point address into a Thumb-mode function pointer
/// by setting the Least Significant Bit (LSB = 1).
///
/// On ARM Cortex-M, the LSB of a branch address indicates Thumb instruction set state.
#[inline(always)]
pub fn make_thumb_entry(addr: usize) -> usize {
    #[cfg(target_arch = "arm")]
    {
        addr | 1
    }
    #[cfg(not(target_arch = "arm"))]
    {
        addr
    }
}
