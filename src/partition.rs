//! PartitionView adapter for sub-partitioning flash storage for sequential-storage and cfg-noodle.

use embedded_storage::nor_flash::{ErrorType, NorFlashError, NorFlashErrorKind};
use embedded_storage_async::nor_flash::{
    NorFlash as AsyncNorFlash, ReadNorFlash as AsyncReadNorFlash,
};

/// Partition boundary error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PartitionError<E> {
    /// Underlying storage driver error.
    Storage(E),
    /// Operation went outside the partition boundary.
    OutOfBounds,
    /// Unaligned offset or length according to flash geometry.
    Misaligned,
}

impl<E: core::fmt::Debug> core::fmt::Display for PartitionError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "Underlying storage error: {e:?}"),
            Self::OutOfBounds => write!(f, "Operation out of partition bounds"),
            Self::Misaligned => write!(f, "Operation misaligned to flash page/sector boundary"),
        }
    }
}

impl<E: NorFlashError> NorFlashError for PartitionError<E> {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::Storage(e) => e.kind(),
            Self::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            Self::Misaligned => NorFlashErrorKind::NotAligned,
        }
    }
}

/// A zero-cost view into a sub-slice of a physical flash memory chip.
///
/// Translates all relative addresses `0..size` into `base_offset..base_offset + size`
/// while strictly enforcing partition boundaries.
#[derive(Debug, Clone)]
pub struct PartitionView<S> {
    storage: S,
    base_offset: u32,
    size: u32,
}

impl<S> PartitionView<S> {
    /// Creates a new partition view.
    ///
    /// # Panics
    /// Panics if `base_offset + size` overflows `u32`.
    pub fn new(storage: S, base_offset: u32, size: u32) -> Self {
        assert!(base_offset.checked_add(size).is_some());
        Self {
            storage,
            base_offset,
            size,
        }
    }

    /// Returns the base offset of this partition within the physical flash.
    pub fn base_offset(&self) -> u32 {
        self.base_offset
    }

    /// Returns the size in bytes of this partition.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Access the underlying storage.
    pub fn inner(&self) -> &S {
        &self.storage
    }

    /// Mutably access the underlying storage.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    /// Consumes the view and returns the inner storage.
    pub fn into_inner(self) -> S {
        self.storage
    }
}

impl<S: ErrorType> ErrorType for PartitionView<S> {
    type Error = PartitionError<S::Error>;
}

impl<S: AsyncReadNorFlash> AsyncReadNorFlash for PartitionView<S> {
    const READ_SIZE: usize = S::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let requested_end = offset
            .checked_add(bytes.len() as u32)
            .ok_or(PartitionError::OutOfBounds)?;

        if requested_end > self.size {
            return Err(PartitionError::OutOfBounds);
        }

        let physical_offset = self.base_offset + offset;
        self.storage
            .read(physical_offset, bytes)
            .await
            .map_err(PartitionError::Storage)
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl<S: AsyncNorFlash> AsyncNorFlash for PartitionView<S> {
    const WRITE_SIZE: usize = S::WRITE_SIZE;
    const ERASE_SIZE: usize = S::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if from > to || to > self.size {
            return Err(PartitionError::OutOfBounds);
        }

        let phys_from = self.base_offset + from;
        let phys_to = self.base_offset + to;

        self.storage
            .erase(phys_from, phys_to)
            .await
            .map_err(PartitionError::Storage)
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let requested_end = offset
            .checked_add(bytes.len() as u32)
            .ok_or(PartitionError::OutOfBounds)?;

        if requested_end > self.size {
            return Err(PartitionError::OutOfBounds);
        }

        let physical_offset = self.base_offset + offset;
        self.storage
            .write(physical_offset, bytes)
            .await
            .map_err(PartitionError::Storage)
    }
}

// Blocking implementations if the underlying storage implements blocking NorFlash
impl<S: embedded_storage::nor_flash::ReadNorFlash> embedded_storage::nor_flash::ReadNorFlash
    for PartitionView<S>
{
    const READ_SIZE: usize = S::READ_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let requested_end = offset
            .checked_add(bytes.len() as u32)
            .ok_or(PartitionError::OutOfBounds)?;

        if requested_end > self.size {
            return Err(PartitionError::OutOfBounds);
        }

        let physical_offset = self.base_offset + offset;
        self.storage
            .read(physical_offset, bytes)
            .map_err(PartitionError::Storage)
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl<S: embedded_storage::nor_flash::NorFlash> embedded_storage::nor_flash::NorFlash
    for PartitionView<S>
{
    const WRITE_SIZE: usize = S::WRITE_SIZE;
    const ERASE_SIZE: usize = S::ERASE_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if from > to || to > self.size {
            return Err(PartitionError::OutOfBounds);
        }

        let phys_from = self.base_offset + from;
        let phys_to = self.base_offset + to;

        self.storage
            .erase(phys_from, phys_to)
            .map_err(PartitionError::Storage)
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let requested_end = offset
            .checked_add(bytes.len() as u32)
            .ok_or(PartitionError::OutOfBounds)?;

        if requested_end > self.size {
            return Err(PartitionError::OutOfBounds);
        }

        let physical_offset = self.base_offset + offset;
        self.storage
            .write(physical_offset, bytes)
            .map_err(PartitionError::Storage)
    }
}
