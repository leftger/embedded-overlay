//! LRU Sector cache in SRAM for streamable VFS.

use crate::error::VfsError;
use embedded_storage_async::nor_flash::ReadNorFlash;

/// A cached sector slot in RAM.
#[derive(Debug)]
struct CacheEntry<const SECTOR_SIZE: usize> {
    sector_id: Option<u32>,
    generation: u64,
    data: [u8; SECTOR_SIZE],
}

/// An LRU cache storing whole flash sectors in microcontroller SRAM.
pub struct LruSectorCache<const SECTORS: usize, const SECTOR_SIZE: usize> {
    entries: [CacheEntry<SECTOR_SIZE>; SECTORS],
    lru_counter: u64,
}

impl<const SECTORS: usize, const SECTOR_SIZE: usize> LruSectorCache<SECTORS, SECTOR_SIZE> {
    /// Creates a new empty sector cache.
    pub const fn new() -> Self {
        // Const-compatible initialization
        const fn make_entry<const SZ: usize>() -> CacheEntry<SZ> {
            CacheEntry {
                sector_id: None,
                generation: 0,
                data: [0u8; SZ],
            }
        }
        let entries = [const { make_entry() }; SECTORS];
        Self {
            entries,
            lru_counter: 0,
        }
    }

    /// Looks up a sector in the cache. If present, returns a reference to its data.
    pub fn get(&mut self, sector_id: u32) -> Option<&[u8; SECTOR_SIZE]> {
        for entry in &mut self.entries {
            if entry.sector_id == Some(sector_id) {
                self.lru_counter = self.lru_counter.wrapping_add(1);
                entry.generation = self.lru_counter;
                return Some(&entry.data);
            }
        }
        None
    }

    /// Fetches a sector, reading it into the cache from storage on cache miss.
    pub async fn get_or_load<S: ReadNorFlash>(
        &mut self,
        storage: &mut S,
        sector_id: u32,
        flash_base_addr: u32,
    ) -> Result<&[u8; SECTOR_SIZE], VfsError> {
        // 1. Check if already present
        for i in 0..SECTORS {
            if self.entries[i].sector_id == Some(sector_id) {
                self.lru_counter = self.lru_counter.wrapping_add(1);
                self.entries[i].generation = self.lru_counter;
                return Ok(&self.entries[i].data);
            }
        }

        // 2. Select slot for eviction (empty slot, or lowest generation)
        let mut target_idx = 0;
        let mut min_gen = u64::MAX;

        for i in 0..SECTORS {
            if self.entries[i].sector_id.is_none() {
                target_idx = i;
                break;
            }
            if self.entries[i].generation < min_gen {
                min_gen = self.entries[i].generation;
                target_idx = i;
            }
        }

        // 3. Read sector from flash into cache buffer via DMA
        let sector_flash_offset = flash_base_addr + (sector_id * SECTOR_SIZE as u32);
        storage
            .read(sector_flash_offset, &mut self.entries[target_idx].data)
            .await
            .map_err(|_| VfsError::Storage)?;

        self.lru_counter = self.lru_counter.wrapping_add(1);
        self.entries[target_idx].sector_id = Some(sector_id);
        self.entries[target_idx].generation = self.lru_counter;

        Ok(&self.entries[target_idx].data)
    }

    /// Invalidates all cached sectors.
    pub fn clear(&mut self) {
        for entry in &mut self.entries {
            entry.sector_id = None;
            entry.generation = 0;
        }
    }
}
