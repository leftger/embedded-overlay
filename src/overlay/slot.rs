//! RAM overlay slot representation and state management.

/// A designated execution slot in microcontroller SRAM.
#[derive(Debug)]
pub struct OverlaySlot {
    /// Physical base address in SRAM.
    pub ram_addr: usize,
    /// Maximum capacity of this slot in bytes.
    pub capacity: usize,
    /// Currently loaded module ID, if any.
    pub resident_module: Option<u32>,
    /// Entry offset of currently resident module.
    pub entry_offset: u32,
    /// LRU access counter.
    pub generation: u64,
    /// If true, this slot is protected from automatic eviction.
    pub is_pinned: bool,
}

impl OverlaySlot {
    /// Creates a new overlay slot targeting a raw physical RAM address.
    ///
    /// # Safety
    /// The caller must ensure that `ram_addr..ram_addr + capacity` is valid,
    /// writable, executable SRAM not overlapping with stack or active static variables.
    pub const unsafe fn new_raw(ram_addr: usize, capacity: usize) -> Self {
        Self {
            ram_addr,
            capacity,
            resident_module: None,
            entry_offset: 0,
            generation: 0,
            is_pinned: false,
        }
    }

    /// Creates an overlay slot safely from a static mutable byte slice.
    pub fn from_static_slice(slice: &'static mut [u8]) -> Self {
        let ram_addr = slice.as_mut_ptr() as usize;
        let capacity = slice.len();
        Self {
            ram_addr,
            capacity,
            resident_module: None,
            entry_offset: 0,
            generation: 0,
            is_pinned: false,
        }
    }

    /// Checks if a given module ID is currently resident in this slot.
    #[inline]
    pub fn is_resident(&self, module_id: u32) -> bool {
        self.resident_module == Some(module_id)
    }

    /// Returns a mutable slice over the RAM buffer of length `len`.
    ///
    /// # Safety
    /// `len` must not exceed `capacity`.
    pub unsafe fn slice_mut(&mut self, len: usize) -> &mut [u8] {
        core::slice::from_raw_parts_mut(self.ram_addr as *mut u8, len)
    }

    /// Evicts the current resident module, marking the slot empty.
    pub fn evict(&mut self) {
        self.resident_module = None;
        self.entry_offset = 0;
    }

    /// Pin the slot to prevent LRU eviction.
    pub fn pin(&mut self) {
        self.is_pinned = true;
    }

    /// Unpin the slot, allowing LRU eviction.
    pub fn unpin(&mut self) {
        self.is_pinned = false;
    }
}
