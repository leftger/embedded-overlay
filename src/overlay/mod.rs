//! RAM-paged code overlay manager with asynchronous DMA streaming.

pub mod module;
pub mod slot;
pub mod sync;

use crate::crc::crc32;
use crate::error::OverlayError;
use crate::header::OverlayHeader;
use embedded_storage_async::nor_flash::ReadNorFlash;
pub use module::{OverlayEntryFn, OverlayModule};
pub use slot::OverlaySlot;
pub use sync::InstructionCacheSync;
#[cfg(all(feature = "cortex-m", target_arch = "arm", target_has_atomic = "ptr"))]
pub use sync::CoreIcacheSync;
#[allow(unused_imports)]
use sync::{make_thumb_entry, validate_thumb_alignment};

/// Manages dynamic loading and execution of code overlays in microcontroller SRAM.
pub struct OverlayManager<S, const SLOTS: usize, B = ()> {
    storage: S,
    slots: [OverlaySlot; SLOTS],
    lru_counter: u64,
    sync: B,
}

impl<S: ReadNorFlash, const SLOTS: usize> OverlayManager<S, SLOTS, ()> {
    /// Creates a new `OverlayManager` using barrier-only instruction synchronization
    /// ([`InstructionCacheSync`] is implemented for `()`).
    ///
    /// On targets whose code bus has a hardware instruction cache (Cortex-M7, or STM32 parts with
    /// the ICACHE block), use [`OverlayManager::with_sync`] so the required cache maintenance runs
    /// after each load.
    pub fn new(storage: S, slots: [OverlaySlot; SLOTS]) -> Self {
        Self {
            storage,
            slots,
            lru_counter: 0,
            sync: (),
        }
    }
}

impl<S: ReadNorFlash, const SLOTS: usize, B: InstructionCacheSync> OverlayManager<S, SLOTS, B> {
    /// Creates a new `OverlayManager` with a caller-provided instruction synchronization hook.
    ///
    /// The hook's [`code_loaded`](InstructionCacheSync::code_loaded) method is invoked once after
    /// each module is streamed into a slot and verified, immediately before the slot is marked
    /// resident.
    pub fn with_sync(storage: S, slots: [OverlaySlot; SLOTS], sync: B) -> Self {
        Self {
            storage,
            slots,
            lru_counter: 0,
            sync,
        }
    }

    /// Returns a reference to the instruction synchronization hook.
    pub fn sync(&self) -> &B {
        &self.sync
    }

    /// Returns mutable access to the instruction synchronization hook.
    pub fn sync_mut(&mut self) -> &mut B {
        &mut self.sync
    }

    /// Access the underlying storage driver.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Mutable access to the underlying storage driver.
    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    /// Returns a reference to all overlay slots.
    pub fn slots(&self) -> &[OverlaySlot; SLOTS] {
        &self.slots
    }

    /// Checks if a module is currently resident in any RAM slot.
    pub fn find_resident_slot(&self, module_id: u32) -> Option<usize> {
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.is_resident(module_id) {
                return Some(i);
            }
        }
        None
    }

    /// Returns true if the specified module is resident in RAM.
    pub fn is_resident(&self, module_id: u32) -> bool {
        self.find_resident_slot(module_id).is_some()
    }

    /// Evicts a module from RAM if it is currently resident.
    pub fn evict(&mut self, module_id: u32) -> bool {
        if let Some(slot_idx) = self.find_resident_slot(module_id) {
            self.slots[slot_idx].evict();
            true
        } else {
            false
        }
    }

    /// Pin a slot to prevent it from being evicted by LRU.
    pub fn pin_slot(&mut self, slot_idx: usize) -> Result<(), OverlayError> {
        let slot = self.slots.get_mut(slot_idx).ok_or(OverlayError::InvalidSlotIndex)?;
        slot.pin();
        Ok(())
    }

    /// Unpin a slot to allow LRU eviction.
    pub fn unpin_slot(&mut self, slot_idx: usize) -> Result<(), OverlayError> {
        let slot = self.slots.get_mut(slot_idx).ok_or(OverlayError::InvalidSlotIndex)?;
        slot.unpin();
        Ok(())
    }

    /// Selects a slot to load into (prefers empty slot, falls back to LRU unpinned slot).
    fn select_eviction_slot(&self) -> Result<usize, OverlayError> {
        // 1. Prefer empty slot
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.resident_module.is_none() && !slot.is_pinned {
                return Ok(i);
            }
        }

        // 2. Select oldest unpinned slot (LRU)
        let mut best_idx = None;
        let mut min_gen = u64::MAX;

        for (i, slot) in self.slots.iter().enumerate() {
            if !slot.is_pinned && slot.generation < min_gen {
                min_gen = slot.generation;
                best_idx = Some(i);
            }
        }

        best_idx.ok_or(OverlayError::InvalidSlotIndex)
    }

    fn next_generation(&mut self) -> u64 {
        self.lru_counter = self.lru_counter.wrapping_add(1);
        self.lru_counter
    }

    /// Ensures that an overlay module is resident in a RAM slot.
    ///
    /// If the module is already loaded, updates LRU tracking and returns the slot index.
    /// If not loaded:
    /// 1. Reads the 32-byte header asynchronously via DMA.
    /// 2. Selects an available or LRU RAM slot.
    /// 3. Streams the machine code into the RAM buffer asynchronously.
    /// 4. Verifies the IEEE 802.3 CRC32 checksum.
    /// 5. Synchronizes instruction fetch via the [`InstructionCacheSync`] hook.
    ///
    /// Returns the slot index containing the resident module.
    pub async fn ensure_resident(
        &mut self,
        module_id: u32,
        flash_offset: u32,
    ) -> Result<usize, OverlayError> {
        if let Some(slot_idx) = self.find_resident_slot(module_id) {
            let gen = self.next_generation();
            self.slots[slot_idx].generation = gen;
            return Ok(slot_idx);
        }

        // Read 32-byte header from external flash
        let mut header_buf = [0u8; OverlayHeader::SIZE];
        self.storage
            .read(flash_offset, &mut header_buf)
            .await
            .map_err(|_| OverlayError::Storage)?;

        let header = OverlayHeader::from_bytes(&header_buf)?;
        if header.module_id != module_id {
            return Err(OverlayError::ModuleNotFound(module_id));
        }

        let slot_idx = self.select_eviction_slot()?;
        let slot = &mut self.slots[slot_idx];

        let code_len = header.code_size as usize;
        if code_len > slot.capacity {
            return Err(OverlayError::SlotTooSmall {
                required: code_len,
                available: slot.capacity,
            });
        }

        // Validate alignment for ARM Cortex-M
        if !validate_thumb_alignment(slot.exec_addr, header.entry_offset as usize) {
            return Err(OverlayError::InvalidAlignment);
        }

        // Stream machine code from external flash directly into SRAM via DMA
        let code_offset = flash_offset + OverlayHeader::SIZE as u32;
        let ram_slice = unsafe { slot.slice_mut(code_len) };
        self.storage
            .read(code_offset, ram_slice)
            .await
            .map_err(|_| OverlayError::Storage)?;

        // Verify payload CRC32
        let calculated_crc = crc32(ram_slice);
        if calculated_crc != header.payload_crc32 {
            slot.evict();
            return Err(OverlayError::PayloadCrcMismatch {
                expected: header.payload_crc32,
                calculated: calculated_crc,
            });
        }

        // Synchronize instruction fetch with the freshly written code. On parts with a hardware
        // instruction cache, the hook performs the cache maintenance that DSB/ISB alone cannot.
        self.sync.code_loaded();

        let next_gen = self.next_generation();
        let slot = &mut self.slots[slot_idx];
        slot.resident_module = Some(module_id);
        slot.entry_offset = header.entry_offset;
        slot.generation = next_gen;

        Ok(slot_idx)
    }

    /// Type-safe version of `ensure_resident` using an [`OverlayModule`] marker type.
    pub async fn ensure_resident_typed<M: OverlayModule>(
        &mut self,
        flash_offset: u32,
    ) -> Result<usize, OverlayError> {
        self.ensure_resident(M::MODULE_ID, flash_offset).await
    }

    /// Invokes the resident module in `slot_idx` with raw arguments.
    ///
    /// # Safety
    /// The caller must ensure that the entry point in `slot_idx` accepts `Args`
    /// and returns `Output` according to the C calling convention.
    pub unsafe fn call_raw<Args, Output>(
        &self,
        slot_idx: usize,
        args: Args,
    ) -> Result<Output, OverlayError> {
        let slot = self.slots.get(slot_idx).ok_or(OverlayError::InvalidSlotIndex)?;
        if slot.resident_module.is_none() {
            return Err(OverlayError::InvalidSlotIndex);
        }

        #[cfg(target_arch = "arm")]
        let entry_addr = make_thumb_entry(slot.exec_addr + slot.entry_offset as usize);

        #[cfg(not(target_arch = "arm"))]
        let entry_addr = if slot.capacity >= core::mem::size_of::<usize>() {
            *(slot.ram_addr as *const usize)
        } else {
            slot.exec_addr + slot.entry_offset as usize
        };

        let entry_fn: OverlayEntryFn<Args, Output> = core::mem::transmute(entry_addr);
        Ok(entry_fn(args))
    }

    /// Invokes the resident module in `slot_idx` with compile-time type safety.
    pub fn call_typed<M: OverlayModule>(
        &self,
        slot_idx: usize,
        args: M::Args,
    ) -> Result<M::Output, OverlayError> {
        let slot = self.slots.get(slot_idx).ok_or(OverlayError::InvalidSlotIndex)?;
        if slot.resident_module != Some(M::MODULE_ID) {
            return Err(OverlayError::ModuleNotFound(M::MODULE_ID));
        }

        unsafe { self.call_raw(slot_idx, args) }
    }

    /// High-level helper: ensures module `M` is resident in RAM and immediately executes it.
    pub async fn execute<M: OverlayModule>(
        &mut self,
        flash_offset: u32,
        args: M::Args,
    ) -> Result<M::Output, OverlayError> {
        let slot_idx = self.ensure_resident_typed::<M>(flash_offset).await?;
        self.call_typed::<M>(slot_idx, args)
    }
}
