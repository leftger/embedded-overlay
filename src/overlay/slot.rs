//! RAM overlay slot representation and state management.

/// A designated execution slot in microcontroller SRAM.
#[derive(Debug)]
pub struct OverlaySlot {
    /// Physical base address in SRAM. Machine code is written here, over the system bus.
    pub ram_addr: usize,
    /// Address the CPU branches to when executing code in this slot.
    ///
    /// Defaults to [`ram_addr`](Self::ram_addr). Set it to an alias address (for example an
    /// ICACHE code-region window, see [`Self::with_exec_addr`]) when instruction fetches should
    /// be served through an instruction cache while writes still target `ram_addr`.
    pub exec_addr: usize,
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
            exec_addr: ram_addr,
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
            exec_addr: ram_addr,
            capacity,
            resident_module: None,
            entry_offset: 0,
            generation: 0,
            is_pinned: false,
        }
    }

    /// Sets a distinct execution address for this slot.
    ///
    /// Use this when instruction fetches should go through an instruction-cache alias window while
    /// code is still written to the physical SRAM address in [`ram_addr`](Self::ram_addr). For
    /// example, on STM32WBA the ICACHE can remap a 2 MB-aligned SRAM region into a code-region
    /// alias, so setting `exec_addr` to that alias routes instruction fetches through the cache.
    ///
    /// `ram_addr` and `exec_addr` must refer to the same physical memory: the code loaded at
    /// `ram_addr` is what executes at `exec_addr`.
    pub fn with_exec_addr(mut self, exec_addr: usize) -> Self {
        self.exec_addr = exec_addr;
        self
    }

    /// Sets the execution address in place. See [`Self::with_exec_addr`].
    pub fn set_exec_addr(&mut self, exec_addr: usize) {
        self.exec_addr = exec_addr;
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
