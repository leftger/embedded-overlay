//! Streamable Virtual Filesystem for high-performance asset streaming from SPI flash.

pub mod cache;

use crate::error::VfsError;
use crate::header::{VfsAssetEntry, VfsSuperblock};
pub use cache::LruSectorCache;
use embedded_storage_async::nor_flash::ReadNorFlash;

/// High-performance streamable VFS reader.
pub struct VfsReader<S> {
    storage: S,
    partition_base: u32,
    superblock: VfsSuperblock,
}

impl<S: ReadNorFlash> VfsReader<S> {
    /// Mounts the VFS by reading and validating the superblock at `partition_base`.
    pub async fn mount(mut storage: S, partition_base: u32) -> Result<Self, VfsError> {
        let mut sb_buf = [0u8; VfsSuperblock::SIZE];
        storage
            .read(partition_base, &mut sb_buf)
            .await
            .map_err(|_| VfsError::Storage)?;

        let superblock = VfsSuperblock::from_bytes(&sb_buf)?;

        Ok(Self {
            storage,
            partition_base,
            superblock,
        })
    }

    /// Access the underlying storage driver.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Mutable access to the underlying storage driver.
    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    /// Returns the total number of assets indexed in the VFS.
    pub fn asset_count(&self) -> usize {
        self.superblock.asset_count as usize
    }

    /// Searches the on-flash index for the given `asset_id`.
    pub async fn find_entry(&mut self, asset_id: u32) -> Result<VfsAssetEntry, VfsError> {
        let count = self.superblock.asset_count as usize;
        let index_start = self.partition_base + self.superblock.index_offset;

        // Read entries in small chunks to keep stack footprint tiny
        let mut entry_buf = [0u8; VfsAssetEntry::SIZE];

        for i in 0..count {
            let offset = index_start + (i * VfsAssetEntry::SIZE) as u32;
            self.storage
                .read(offset, &mut entry_buf)
                .await
                .map_err(|_| VfsError::Storage)?;

            let entry = VfsAssetEntry::from_bytes(&entry_buf);
            if entry.asset_id == asset_id {
                return Ok(entry);
            }
        }

        Err(VfsError::AssetNotFound(asset_id))
    }

    /// Returns the byte length of an asset.
    pub async fn asset_len(&mut self, asset_id: u32) -> Result<usize, VfsError> {
        let entry = self.find_entry(asset_id).await?;
        Ok(entry.length as usize)
    }

    /// Streams a chunk of an asset directly into a destination RAM buffer via DMA.
    ///
    /// Reads up to `dest.len()` bytes starting at `offset` within the asset.
    /// Returns the number of bytes read.
    pub async fn stream_chunk(
        &mut self,
        asset_id: u32,
        offset: usize,
        dest: &mut [u8],
    ) -> Result<usize, VfsError> {
        let entry = self.find_entry(asset_id).await?;
        let asset_len = entry.length as usize;

        if offset >= asset_len {
            return Ok(0);
        }

        let to_read = (asset_len - offset).min(dest.len());
        let flash_addr =
            self.partition_base + self.superblock.data_offset + entry.flash_offset + offset as u32;

        self.storage
            .read(flash_addr, &mut dest[..to_read])
            .await
            .map_err(|_| VfsError::Storage)?;

        Ok(to_read)
    }

    /// Reads the complete asset into `dest`. Returns error if `dest` is too small.
    pub async fn read_asset(&mut self, asset_id: u32, dest: &mut [u8]) -> Result<usize, VfsError> {
        let entry = self.find_entry(asset_id).await?;
        let asset_len = entry.length as usize;

        if dest.len() < asset_len {
            return Err(VfsError::BufferTooSmall {
                required: asset_len,
                provided: dest.len(),
            });
        }

        self.stream_chunk(asset_id, 0, &mut dest[..asset_len]).await
    }
}
