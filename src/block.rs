//! Block device abstraction and adapter for SD cards, microSD, and eMMC.
//!
//! Bridges 512-byte sector block devices (such as microSD cards over SPI)
//! to the asynchronous `ReadNorFlash` and `NorFlash` traits used by
//! [`OverlayManager`](crate::OverlayManager) and [`VfsReader`](crate::VfsReader).

use core::fmt;
use embedded_storage::nor_flash::{ErrorType, NorFlashError, NorFlashErrorKind};
use embedded_storage_async::nor_flash::{
    NorFlash as AsyncNorFlash, ReadNorFlash as AsyncReadNorFlash,
};

/// Trait representing an asynchronous block storage device (such as an SD/microSD card).
pub trait AsyncBlockDevice {
    type Error: fmt::Debug;

    /// Block size in bytes (standard SD cards are 512 bytes).
    const BLOCK_SIZE: usize = 512;

    /// Reads one or more contiguous 512-byte blocks into `buffer`.
    ///
    /// # Panics
    /// May panic if `buffer.len()` is not a multiple of `BLOCK_SIZE`.
    fn read_blocks<'a>(
        &'a mut self,
        start_block: u32,
        buffer: &'a mut [u8],
    ) -> impl core::future::Future<Output = Result<(), Self::Error>> + 'a;

    /// Writes one or more contiguous 512-byte blocks from `buffer`.
    ///
    /// # Panics
    /// May panic if `buffer.len()` is not a multiple of `BLOCK_SIZE`.
    fn write_blocks<'a>(
        &'a mut self,
        start_block: u32,
        buffer: &'a [u8],
    ) -> impl core::future::Future<Output = Result<(), Self::Error>> + 'a;

    /// Total number of blocks available on the card.
    fn num_blocks(&self) -> u64;
}

/// Errors occurring during block device operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BlockAdapterError<E> {
    Device(E),
    OutOfBounds,
    InvalidAlignment,
}

impl<E: fmt::Debug> fmt::Display for BlockAdapterError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(e) => write!(f, "Block device error: {e:?}"),
            Self::OutOfBounds => write!(f, "Operation out of block device bounds"),
            Self::InvalidAlignment => write!(f, "Unaligned block operation"),
        }
    }
}

impl<E: fmt::Debug> NorFlashError for BlockAdapterError<E> {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::Device(_) => NorFlashErrorKind::Other,
            Self::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            Self::InvalidAlignment => NorFlashErrorKind::NotAligned,
        }
    }
}

/// An adapter that wraps any 512-byte [`AsyncBlockDevice`] (such as a microSD card)
/// and implements [`AsyncReadNorFlash`] and [`AsyncNorFlash`].
///
/// Features:
/// - **Zero-Copy DMA Path**: When reading or writing whole 512-byte blocks,
///   streams data directly between RAM and the SD card over SPI DMA.
/// - **Single-Sector Bounce Buffer**: Seamlessly handles arbitrary byte offsets
///   and sub-block reads (such as 32-byte overlay headers or 16-byte VFS index records).
pub struct BlockDeviceAdapter<B, const BLOCK_SIZE: usize = 512> {
    device: B,
    cached_block: Option<u32>,
    cache: [u8; BLOCK_SIZE],
}

impl<B, const BLOCK_SIZE: usize> BlockDeviceAdapter<B, BLOCK_SIZE> {
    /// Creates a new adapter wrapping the given block device.
    pub const fn new(device: B) -> Self {
        Self {
            device,
            cached_block: None,
            cache: [0u8; BLOCK_SIZE],
        }
    }

    /// Access the underlying block device.
    pub fn device(&self) -> &B {
        &self.device
    }

    /// Mutably access the underlying block device.
    pub fn device_mut(&mut self) -> &mut B {
        &mut self.device
    }

    /// Consumes the adapter and returns the underlying device.
    pub fn into_device(self) -> B {
        self.device
    }
}

impl<B: AsyncBlockDevice, const BLOCK_SIZE: usize> ErrorType for BlockDeviceAdapter<B, BLOCK_SIZE> {
    type Error = BlockAdapterError<B::Error>;
}

impl<B: AsyncBlockDevice, const BLOCK_SIZE: usize> AsyncReadNorFlash
    for BlockDeviceAdapter<B, BLOCK_SIZE>
{
    const READ_SIZE: usize = 1;

    async fn read(&mut self, mut offset: u32, mut dest: &mut [u8]) -> Result<(), Self::Error> {
        let total_capacity = (self.device.num_blocks() * BLOCK_SIZE as u64) as u32;
        let requested_end = offset
            .checked_add(dest.len() as u32)
            .ok_or(BlockAdapterError::OutOfBounds)?;

        if requested_end > total_capacity {
            return Err(BlockAdapterError::OutOfBounds);
        }

        while !dest.is_empty() {
            let block_idx = offset / BLOCK_SIZE as u32;
            let block_offset = (offset % BLOCK_SIZE as u32) as usize;

            if block_offset == 0 && dest.len() >= BLOCK_SIZE {
                // Direct zero-copy DMA block read!
                let whole_blocks = dest.len() / BLOCK_SIZE;
                let bytes_to_read = whole_blocks * BLOCK_SIZE;

                self.device
                    .read_blocks(block_idx, &mut dest[..bytes_to_read])
                    .await
                    .map_err(BlockAdapterError::Device)?;

                offset += bytes_to_read as u32;
                dest = &mut dest[bytes_to_read..];
            } else {
                // Unaligned sub-block read: use internal sector bounce buffer
                if self.cached_block != Some(block_idx) {
                    self.device
                        .read_blocks(block_idx, &mut self.cache)
                        .await
                        .map_err(BlockAdapterError::Device)?;
                    self.cached_block = Some(block_idx);
                }

                let available_in_block = BLOCK_SIZE - block_offset;
                let chunk_len = dest.len().min(available_in_block);

                dest[..chunk_len]
                    .copy_from_slice(&self.cache[block_offset..block_offset + chunk_len]);

                offset += chunk_len as u32;
                dest = &mut dest[chunk_len..];
            }
        }

        Ok(())
    }

    fn capacity(&self) -> usize {
        (self.device.num_blocks() * BLOCK_SIZE as u64) as usize
    }
}

impl<B: AsyncBlockDevice, const BLOCK_SIZE: usize> AsyncNorFlash
    for BlockDeviceAdapter<B, BLOCK_SIZE>
{
    const WRITE_SIZE: usize = 1;
    const ERASE_SIZE: usize = BLOCK_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        // SD cards feature internal flash management and do not require pre-erasing
        if from > to || to as u64 > self.device.num_blocks() * BLOCK_SIZE as u64 {
            return Err(BlockAdapterError::OutOfBounds);
        }
        Ok(())
    }

    async fn write(&mut self, mut offset: u32, mut src: &[u8]) -> Result<(), Self::Error> {
        let total_capacity = (self.device.num_blocks() * BLOCK_SIZE as u64) as u32;
        let requested_end = offset
            .checked_add(src.len() as u32)
            .ok_or(BlockAdapterError::OutOfBounds)?;

        if requested_end > total_capacity {
            return Err(BlockAdapterError::OutOfBounds);
        }

        while !src.is_empty() {
            let block_idx = offset / BLOCK_SIZE as u32;
            let block_offset = (offset % BLOCK_SIZE as u32) as usize;

            if block_offset == 0 && src.len() >= BLOCK_SIZE {
                // Direct aligned block write
                let whole_blocks = src.len() / BLOCK_SIZE;
                let bytes_to_write = whole_blocks * BLOCK_SIZE;

                self.device
                    .write_blocks(block_idx, &src[..bytes_to_write])
                    .await
                    .map_err(BlockAdapterError::Device)?;

                // Invalidate cached block if overwritten
                if let Some(cached) = self.cached_block {
                    if cached >= block_idx && cached < block_idx + whole_blocks as u32 {
                        self.cached_block = None;
                    }
                }

                offset += bytes_to_write as u32;
                src = &src[bytes_to_write..];
            } else {
                // Unaligned sub-block write (Read-Modify-Write)
                if self.cached_block != Some(block_idx) {
                    self.device
                        .read_blocks(block_idx, &mut self.cache)
                        .await
                        .map_err(BlockAdapterError::Device)?;
                    self.cached_block = Some(block_idx);
                }

                let available = BLOCK_SIZE - block_offset;
                let chunk_len = src.len().min(available);

                self.cache[block_offset..block_offset + chunk_len].copy_from_slice(&src[..chunk_len]);

                // Write modified block back to device
                self.device
                    .write_blocks(block_idx, &self.cache)
                    .await
                    .map_err(BlockAdapterError::Device)?;

                offset += chunk_len as u32;
                src = &src[chunk_len..];
            }
        }

        Ok(())
    }
}
