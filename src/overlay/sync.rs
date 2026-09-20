//! Processor memory barriers and instruction-memory synchronization for dynamic code execution.
//!
//! Two layers live here:
//! 1. [`sync_instruction_memory`] - the core pipeline barriers (`DSB` + `ISB`).
//! 2. [`InstructionCacheSync`] - the hardware hook the overlay engine invokes after loading
//!    code, so targets with a real instruction cache (Cortex-M7, or the STM32 ICACHE block) can
//!    add the cache maintenance they require.

/// Synchronizes the processor pipeline with newly written machine code using core barriers only.
///
/// On ARM Cortex-M targets, this executes:
/// 1. `dsb` (Data Synchronization Barrier) - Ensures all memory stores are committed.
/// 2. `isb` (Instruction Synchronization Barrier) - Flushes the processor pipeline so
///    that subsequently fetched instructions observe the new code in RAM.
///
/// # Hardware scope
///
/// These barriers are sufficient on cores with no cache in front of the code bus
/// (Cortex-M0/M0+/M3/M4). They do **not** invalidate a hardware instruction cache: on a
/// Cortex-M7 (core I-cache) or an STM32 with the ICACHE block, a cache maintenance operation is
/// also required. [`InstructionCacheSync`] is the hook for that, and its blanket implementation
/// for `()` calls this function.
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

/// Hardware hook that makes freshly written machine code visible to instruction fetch.
///
/// [`OverlayManager`](crate::overlay::OverlayManager) invokes
/// [`code_loaded`](InstructionCacheSync::code_loaded) exactly once, after a module's machine code
/// has been streamed into a RAM slot and its CRC verified, and before the slot is marked resident
/// or invoked.
///
/// This trait keeps `embedded-overlay` hardware-agnostic: the register-level cache driver belongs
/// in the HAL (for example `embassy_stm32::icache::Icache`), while this crate only defines *when*
/// synchronization must happen.
///
/// # Implementations
///
/// * `()` - the default, used by
///   [`OverlayManager::new`](crate::overlay::OverlayManager::new). Runs `DSB` + `ISB` via
///   [`sync_instruction_memory`]. Correct for Cortex-M0/M0+/M3/M4.
/// * Cortex-M7 with the core I-cache: use [`CoreIcacheSync`] (available when the `cortex-m`
///   feature is enabled on Armv7-M/Armv8-M targets).
/// * STM32 parts with the ICACHE block (e.g. WBA/U5/H5): `DSB`, then
///   `embassy_stm32::icache::Icache::invalidate()` (a full-cache `CACHEINV`), then `ISB`. The
///   core barriers alone never invalidate that block. See `demos/wba65-overlay-demo` for a
///   reference implementation.
///
/// # Correctness
///
/// Implementations must fully synchronize instruction fetch with the stores that populated the
/// slot. Returning without doing so may execute stale instructions.
pub trait InstructionCacheSync {
    /// Called after a slot has been populated with new machine code.
    fn code_loaded(&mut self);
}

impl InstructionCacheSync for () {
    #[inline(always)]
    fn code_loaded(&mut self) {
        sync_instruction_memory();
    }
}

/// Instruction-cache synchronization for cores with the architectural I-cache invalidation
/// register (`SCB::ICIALLU`), most notably **Cortex-M7**.
///
/// Equivalent to the default `()` implementation plus a full core I-cache invalidate:
/// `DSB` -> `ICIALLU` -> `DSB` -> `ISB`.
///
/// # Availability
///
/// `CoreIcacheSync` is compiled when the `cortex-m` feature is enabled and the target is
/// Armv7-M or Armv8-M. It is deliberately absent on Armv6-M (Cortex-M0/M0+), where the `ICIALLU`
/// register does not exist: the `target_has_atomic = "ptr"` bound below mirrors `cortex-m`'s own
/// `not(armv6m)` gating for the cache-maintenance API.
///
/// ```ignore
/// use embedded_overlay::{CoreIcacheSync, OverlayManager};
///
/// let manager = OverlayManager::<_, 2, _>::with_sync(flash, slots, CoreIcacheSync);
/// ```
///
/// # Cortex-M7 D-cache caveat
///
/// If the D-cache is enabled and the slot was populated by **CPU stores** rather than DMA, dirty
/// D-cache lines must be cleaned before this runs, otherwise the I-cache refill can read stale
/// SRAM. The recommended setup is to map the overlay slot region as **non-cacheable** (or at
/// least non-write-back) in the MPU, which makes `ICIALLU` sufficient on its own; alternatively
/// clean the D-cache for the written range first.
///
/// This does **not** cover the external ICACHE block on STM32WBA/U5/H5, which needs its own
/// `CACHEINV` maintenance (see `demos/wba65-overlay-demo`).
#[cfg(all(feature = "cortex-m", target_arch = "arm", target_has_atomic = "ptr"))]
pub struct CoreIcacheSync;

#[cfg(all(feature = "cortex-m", target_arch = "arm", target_has_atomic = "ptr"))]
impl InstructionCacheSync for CoreIcacheSync {
    #[inline]
    fn code_loaded(&mut self) {
        // Commit the stores that populated the slot before invalidating the I-cache.
        cortex_m::asm::dsb();

        // SAFETY: `Peripherals::steal()` only aliases if a caller concurrently holds a distinct
        // handle to the same peripheral. The SCB is a stateless handle over a fixed MMIO block,
        // and `invalidate_icache` only writes the write-only ICIALLU register followed by
        // DSB/ISB. We take a fresh handle per call and never retain it.
        let mut scb = unsafe { cortex_m::peripheral::Peripherals::steal() }.SCB;

        // Full core I-cache invalidate (ICIALLU), followed by DSB + ISB.
        scb.invalidate_icache();
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
