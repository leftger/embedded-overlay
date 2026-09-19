//! Embassy-native Overlay Engine with async mutex protection and automatic module dispatch.

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::mutex::Mutex;
use embedded_storage_async::nor_flash::ReadNorFlash;

use crate::error::OverlayError;
use crate::overlay::{OverlayManager, OverlayModule, OverlaySlot};

/// Module registration record mapping a 32-bit module ID to its flash offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleRegistration {
    /// Unique 32-bit module identifier.
    pub module_id: u32,
    /// Byte offset where the `.ovl` container begins in storage.
    pub flash_offset: u32,
}

/// An Embassy-native, thread-safe, task-sharable Overlay Engine.
///
/// Wraps [`OverlayManager`] in an Embassy async [`Mutex`] and maintains an on-chip
/// module registry mapping `module_id -> flash_offset`.
///
/// This provides transparent overlay invocation: tasks simply call `engine.call::<M>(args).await`
/// without having to know or manage RAM slots, generations, or flash byte offsets.
pub struct EmbassyOverlayEngine<M: RawMutex, S, const SLOTS: usize, const MAX_MODULES: usize = 16> {
    manager: Mutex<M, OverlayManager<S, SLOTS>>,
    registry: [Option<ModuleRegistration>; MAX_MODULES],
}

impl<M: RawMutex, S: ReadNorFlash, const SLOTS: usize, const MAX_MODULES: usize>
    EmbassyOverlayEngine<M, S, SLOTS, MAX_MODULES>
{
    /// Creates a new EmbassyOverlayEngine.
    pub fn new(storage: S, slots: [OverlaySlot; SLOTS]) -> Self {
        Self {
            manager: Mutex::new(OverlayManager::new(storage, slots)),
            registry: [None; MAX_MODULES],
        }
    }

    /// Automatically mounts and discovers all overlays starting at `base_offset` in storage.
    ///
    /// Transparently detects either:
    /// 1. An [`OverlayDirectory`](crate::header::OverlayDirectory) table (`OVLD`).
    /// 2. Sequential [`OverlayHeader`](crate::header::OverlayHeader) containers (`OVL1`).
    ///
    /// This eliminates all manual offset and ID registration boilerplate. Returns the number
    /// of discovered and registered overlay modules.
    pub async fn mount(&mut self, base_offset: u32) -> Result<usize, OverlayError> {
        use crate::header::{
            OverlayDirectory, OverlayDirectoryEntry, OverlayHeader, OVERLAY_DIR_MAGIC,
            OVERLAY_MAGIC,
        };

        let mut discovered = [(0u32, 0u32); MAX_MODULES];
        let mut count = 0;

        {
            let mut mgr = self.manager.lock().await;
            let mut header_buf = [0u8; 32];
            mgr.storage_mut()
                .read(base_offset, &mut header_buf)
                .await
                .map_err(|_| OverlayError::Storage)?;

            let magic = [header_buf[0], header_buf[1], header_buf[2], header_buf[3]];

            if magic == OVERLAY_DIR_MAGIC {
                let dir = OverlayDirectory::from_bytes(&header_buf)?;
                let mut entry_buf = [0u8; OverlayDirectoryEntry::SIZE];
                let entries_start = base_offset + OverlayDirectory::SIZE as u32;

                for i in 0..dir.entry_count {
                    if count >= MAX_MODULES {
                        break;
                    }
                    let entry_offset =
                        entries_start + (i as u32 * OverlayDirectoryEntry::SIZE as u32);
                    mgr.storage_mut()
                        .read(entry_offset, &mut entry_buf)
                        .await
                        .map_err(|_| OverlayError::Storage)?;

                    let entry = OverlayDirectoryEntry::from_bytes(&entry_buf);
                    discovered[count] = (entry.module_id, entry.flash_offset);
                    count += 1;
                }
            } else if magic == OVERLAY_MAGIC {
                let mut current_offset = base_offset;
                loop {
                    if count >= MAX_MODULES {
                        break;
                    }
                    let header = match OverlayHeader::from_bytes(&header_buf) {
                        Ok(h) => h,
                        Err(_) => break,
                    };

                    discovered[count] = (header.module_id, current_offset);
                    count += 1;

                    // Advance to next overlay: header (32 bytes) + code_size, aligned to 4 bytes
                    let next_step = (OverlayHeader::SIZE as u32 + header.code_size + 3) & !3;
                    current_offset += next_step;

                    // Attempt to read next header
                    if mgr
                        .storage_mut()
                        .read(current_offset, &mut header_buf)
                        .await
                        .is_err()
                    {
                        break;
                    }
                    if [header_buf[0], header_buf[1], header_buf[2], header_buf[3]] != OVERLAY_MAGIC
                    {
                        break;
                    }
                }
            } else {
                return Err(OverlayError::InvalidMagic);
            }
        }

        for i in 0..count {
            self.register_module(discovered[i].0, discovered[i].1)?;
        }

        Ok(count)
    }

    /// Registers a module with its corresponding offset in external flash / SD card.
    pub fn register_module(
        &mut self,
        module_id: u32,
        flash_offset: u32,
    ) -> Result<(), OverlayError> {
        for slot in self.registry.iter_mut() {
            if let Some(entry) = slot {
                if entry.module_id == module_id {
                    entry.flash_offset = flash_offset;
                    return Ok(());
                }
            } else {
                *slot = Some(ModuleRegistration {
                    module_id,
                    flash_offset,
                });
                return Ok(());
            }
        }
        Err(OverlayError::Storage)
    }

    /// Bulk registers a list of module entries.
    pub fn register_modules(
        &mut self,
        modules: &[ModuleRegistration],
    ) -> Result<(), OverlayError> {
        for m in modules {
            self.register_module(m.module_id, m.flash_offset)?;
        }
        Ok(())
    }

    /// Looks up the flash offset for a given module ID.
    pub fn find_offset(&self, module_id: u32) -> Option<u32> {
        for entry in self.registry.iter().flatten() {
            if entry.module_id == module_id {
                return Some(entry.flash_offset);
            }
        }
        None
    }

    /// Transparently executes module `Mod`.
    ///
    /// 1. Resolves `Mod::MODULE_ID` in the registry to find its flash offset.
    /// 2. Asynchronously locks the overlay manager mutex (yielding CPU to other Embassy tasks).
    /// 3. Ensures the module is resident in RAM (streaming via DMA if not already cached).
    /// 4. Synchronizes Thumb-2 memory barriers (`DSB`/`ISB`).
    /// 5. Invokes the resident module with typed arguments and returns the typed output.
    pub async fn call<Mod: OverlayModule>(
        &self,
        args: Mod::Args,
    ) -> Result<Mod::Output, OverlayError> {
        let offset = self
            .find_offset(Mod::MODULE_ID)
            .ok_or(OverlayError::ModuleNotFound(Mod::MODULE_ID))?;

        let mut mgr = self.manager.lock().await;
        let slot_idx = mgr.ensure_resident_typed::<Mod>(offset).await?;
        mgr.call_typed::<Mod>(slot_idx, args)
    }

    /// Asynchronously prefetches module `Mod` into an available or LRU RAM slot in the background.
    ///
    /// Ideal for ping-pong double-buffered execution: stream the next module over SPI DMA
    /// while the CPU runs concurrent tasks.
    pub async fn prefetch<Mod: OverlayModule>(&self) -> Result<usize, OverlayError> {
        let offset = self
            .find_offset(Mod::MODULE_ID)
            .ok_or(OverlayError::ModuleNotFound(Mod::MODULE_ID))?;

        let mut mgr = self.manager.lock().await;
        mgr.ensure_resident_typed::<Mod>(offset).await
    }

    /// Returns true if module `Mod` is currently resident in a RAM slot.
    pub async fn is_resident<Mod: OverlayModule>(&self) -> bool {
        let mgr = self.manager.lock().await;
        mgr.is_resident(Mod::MODULE_ID)
    }

    /// Evicts module `Mod` from RAM.
    pub async fn evict<Mod: OverlayModule>(&self) -> bool {
        let mut mgr = self.manager.lock().await;
        mgr.evict(Mod::MODULE_ID)
    }

    /// Provides access to the underlying Mutex-protected [`OverlayManager`].
    pub fn inner(&self) -> &Mutex<M, OverlayManager<S, SLOTS>> {
        &self.manager
    }
}
