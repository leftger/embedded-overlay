//! Shared storage bus enabling concurrent access to external flash / SD card
//! across multiple Embassy tasks (overlays, VFS streaming, and sequential-storage).

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::mutex::Mutex;
use embedded_storage_async::nor_flash::{
    ErrorType, NorFlash as AsyncNorFlash, ReadNorFlash as AsyncReadNorFlash,
};

/// Shared bus hosting an underlying storage driver behind an Embassy async Mutex.
pub struct SharedStorageBus<M: RawMutex, S> {
    mutex: Mutex<M, S>,
    capacity: usize,
}

impl<M: RawMutex, S: AsyncReadNorFlash> SharedStorageBus<M, S> {
    /// Creates a new SharedStorageBus wrapping the underlying storage driver.
    pub fn new(storage: S) -> Self {
        let capacity = storage.capacity();
        Self {
            mutex: Mutex::new(storage),
            capacity,
        }
    }

    /// Obtains a new handle into this shared storage bus.
    pub fn handle(&self) -> SharedStorageHandle<'_, M, S> {
        SharedStorageHandle { bus: self }
    }

    /// Access the capacity of the storage.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Direct access to the internal mutex for custom locking.
    pub fn mutex(&self) -> &Mutex<M, S> {
        &self.mutex
    }
}

/// A lightweight, copyable handle to a [`SharedStorageBus`].
///
/// Implements [`AsyncReadNorFlash`] and [`AsyncNorFlash`], acquiring the bus mutex
/// for each operation. This allows multiple subsystems (e.g. `OverlayManager`,
/// `VfsReader`, and `PartitionView` for `sequential-storage`) to safely share the
/// same SPI flash or SD card concurrently without ownership transfer.
pub struct SharedStorageHandle<'a, M: RawMutex, S> {
    bus: &'a SharedStorageBus<M, S>,
}

impl<'a, M: RawMutex, S> Clone for SharedStorageHandle<'a, M, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, M: RawMutex, S> Copy for SharedStorageHandle<'a, M, S> {}

impl<'a, M: RawMutex, S: ErrorType> ErrorType for SharedStorageHandle<'a, M, S> {
    type Error = S::Error;
}

impl<'a, M: RawMutex, S: AsyncReadNorFlash> AsyncReadNorFlash for SharedStorageHandle<'a, M, S> {
    const READ_SIZE: usize = S::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let mut guard = self.bus.mutex.lock().await;
        guard.read(offset, bytes).await
    }

    fn capacity(&self) -> usize {
        self.bus.capacity
    }
}

impl<'a, M: RawMutex, S: AsyncNorFlash> AsyncNorFlash for SharedStorageHandle<'a, M, S> {
    const WRITE_SIZE: usize = S::WRITE_SIZE;
    const ERASE_SIZE: usize = S::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let mut guard = self.bus.mutex.lock().await;
        guard.erase(from, to).await
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let mut guard = self.bus.mutex.lock().await;
        guard.write(offset, bytes).await
    }
}
