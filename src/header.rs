//! Binary headers and index structures for overlays and streamable VFS.

use crate::crc::crc32;
use crate::error::{OverlayError, VfsError};

/// Magic bytes for code overlays: "OVL1".
pub const OVERLAY_MAGIC: [u8; 4] = *b"OVL1";
/// Format version for code overlays.
pub const OVERLAY_VERSION: u16 = 1;

/// 32-byte header placed before every overlay code payload in flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(4))]
pub struct OverlayHeader {
    /// Magic signature: `b"OVL1"`.
    pub magic: [u8; 4],
    /// Format version (1).
    pub version: u16,
    /// Overlay flags (bit 0: PIC, bit 1: compressed).
    pub flags: u16,
    /// Unique module identifier.
    pub module_id: u32,
    /// Length of code payload in bytes.
    pub code_size: u32,
    /// Byte offset from start of payload to entry point function.
    pub entry_offset: u32,
    /// Target RAM address (0 if position-independent).
    pub ram_target_addr: u32,
    /// IEEE 802.3 CRC32 of the machine code payload.
    pub payload_crc32: u32,
    /// IEEE 802.3 CRC32 of this header (first 28 bytes).
    pub header_crc32: u32,
}

impl OverlayHeader {
    pub const SIZE: usize = core::mem::size_of::<Self>(); // 32 bytes

    /// Creates a new overlay header with computed header CRC.
    pub fn new(
        module_id: u32,
        code_size: u32,
        entry_offset: u32,
        ram_target_addr: u32,
        payload_crc32: u32,
    ) -> Self {
        let mut header = Self {
            magic: OVERLAY_MAGIC,
            version: OVERLAY_VERSION,
            flags: 0,
            module_id,
            code_size,
            entry_offset,
            ram_target_addr,
            payload_crc32,
            header_crc32: 0,
        };
        let bytes = header.as_bytes();
        header.header_crc32 = crc32(&bytes[..28]);
        header
    }

    /// Serializes the header into a 32-byte array.
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..12].copy_from_slice(&self.module_id.to_le_bytes());
        buf[12..16].copy_from_slice(&self.code_size.to_le_bytes());
        buf[16..20].copy_from_slice(&self.entry_offset.to_le_bytes());
        buf[20..24].copy_from_slice(&self.ram_target_addr.to_le_bytes());
        buf[24..28].copy_from_slice(&self.payload_crc32.to_le_bytes());
        buf[28..32].copy_from_slice(&self.header_crc32.to_le_bytes());
        buf
    }

    /// Deserializes and validates the header from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OverlayError> {
        if bytes.len() < Self::SIZE {
            return Err(OverlayError::Storage);
        }
        let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != OVERLAY_MAGIC {
            return Err(OverlayError::InvalidMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != OVERLAY_VERSION {
            return Err(OverlayError::UnsupportedVersion);
        }
        let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        let module_id = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let code_size = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let entry_offset = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let ram_target_addr = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        let payload_crc32 = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        let header_crc32 = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);

        let calculated_crc = crc32(&bytes[..28]);
        if calculated_crc != header_crc32 {
            return Err(OverlayError::HeaderChecksumMismatch);
        }

        Ok(Self {
            magic,
            version,
            flags,
            module_id,
            code_size,
            entry_offset,
            ram_target_addr,
            payload_crc32,
            header_crc32,
        })
    }

    fn as_bytes(&self) -> [u8; Self::SIZE] {
        self.to_bytes()
    }
}

/// Magic bytes for VFS: "VFS1".
pub const VFS_MAGIC: [u8; 4] = *b"VFS1";
/// Format version for VFS.
pub const VFS_VERSION: u16 = 1;

/// 32-byte Superblock for streamable VFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(4))]
pub struct VfsSuperblock {
    pub magic: [u8; 4],
    pub version: u16,
    pub asset_count: u16,
    pub index_offset: u32,
    pub data_offset: u32,
    pub total_bytes: u32,
    pub crc32: u32,
    pub reserved: [u32; 2],
}

impl VfsSuperblock {
    pub const SIZE: usize = core::mem::size_of::<Self>(); // 32 bytes

    pub fn new(asset_count: u16, index_offset: u32, data_offset: u32, total_bytes: u32) -> Self {
        let mut sb = Self {
            magic: VFS_MAGIC,
            version: VFS_VERSION,
            asset_count,
            index_offset,
            data_offset,
            total_bytes,
            crc32: 0,
            reserved: [0; 2],
        };
        let bytes = sb.to_bytes();
        sb.crc32 = crc32(&bytes[..20]);
        sb
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.asset_count.to_le_bytes());
        buf[8..12].copy_from_slice(&self.index_offset.to_le_bytes());
        buf[12..16].copy_from_slice(&self.data_offset.to_le_bytes());
        buf[16..20].copy_from_slice(&self.total_bytes.to_le_bytes());
        buf[20..24].copy_from_slice(&self.crc32.to_le_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VfsError> {
        if bytes.len() < Self::SIZE {
            return Err(VfsError::Storage);
        }
        let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != VFS_MAGIC {
            return Err(VfsError::InvalidMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != VFS_VERSION {
            return Err(VfsError::UnsupportedVersion);
        }
        let asset_count = u16::from_le_bytes([bytes[6], bytes[7]]);
        let index_offset = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let data_offset = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let total_bytes = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let stored_crc = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);

        let calculated_crc = crc32(&bytes[..20]);
        if calculated_crc != stored_crc {
            return Err(VfsError::SuperblockCrcMismatch);
        }

        Ok(Self {
            magic,
            version,
            asset_count,
            index_offset,
            data_offset,
            total_bytes,
            crc32: stored_crc,
            reserved: [0; 2],
        })
    }
}

/// 16-byte Asset index record within VFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(4))]
pub struct VfsAssetEntry {
    pub asset_id: u32,
    pub flash_offset: u32,
    pub length: u32,
    pub flags: u16,
    pub crc16: u16,
}

impl VfsAssetEntry {
    pub const SIZE: usize = core::mem::size_of::<Self>(); // 16 bytes

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.asset_id.to_le_bytes());
        buf[4..8].copy_from_slice(&self.flash_offset.to_le_bytes());
        buf[8..12].copy_from_slice(&self.length.to_le_bytes());
        buf[12..14].copy_from_slice(&self.flags.to_le_bytes());
        buf[14..16].copy_from_slice(&self.crc16.to_le_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let asset_id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let flash_offset = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let length = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let flags = u16::from_le_bytes([bytes[12], bytes[13]]);
        let crc16 = u16::from_le_bytes([bytes[14], bytes[15]]);
        Self {
            asset_id,
            flash_offset,
            length,
            flags,
            crc16,
        }
    }
}

/// Magic bytes for Overlay Directory: "OVLD".
pub const OVERLAY_DIR_MAGIC: [u8; 4] = *b"OVLD";
/// Format version for Overlay Directory.
pub const OVERLAY_DIR_VERSION: u16 = 1;

/// 32-byte directory header placed at the beginning of an overlay partition or table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(4))]
pub struct OverlayDirectory {
    pub magic: [u8; 4],
    pub version: u16,
    pub entry_count: u16,
    pub index_offset: u32,
    pub total_bytes: u32,
    pub crc32: u32,
    pub reserved: [u32; 3],
}

impl OverlayDirectory {
    pub const SIZE: usize = core::mem::size_of::<Self>(); // 32 bytes

    pub fn new(entry_count: u16, index_offset: u32, total_bytes: u32) -> Self {
        let mut dir = Self {
            magic: OVERLAY_DIR_MAGIC,
            version: OVERLAY_DIR_VERSION,
            entry_count,
            index_offset,
            total_bytes,
            crc32: 0,
            reserved: [0; 3],
        };
        let bytes = dir.to_bytes();
        dir.crc32 = crc32(&bytes[..16]);
        dir
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.entry_count.to_le_bytes());
        buf[8..12].copy_from_slice(&self.index_offset.to_le_bytes());
        buf[12..16].copy_from_slice(&self.total_bytes.to_le_bytes());
        buf[16..20].copy_from_slice(&self.crc32.to_le_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OverlayError> {
        if bytes.len() < Self::SIZE {
            return Err(OverlayError::Storage);
        }
        let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != OVERLAY_DIR_MAGIC {
            return Err(OverlayError::InvalidMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != OVERLAY_DIR_VERSION {
            return Err(OverlayError::UnsupportedVersion);
        }
        let entry_count = u16::from_le_bytes([bytes[6], bytes[7]]);
        let index_offset = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let total_bytes = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let stored_crc = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);

        let calculated_crc = crc32(&bytes[..16]);
        if calculated_crc != stored_crc {
            return Err(OverlayError::HeaderChecksumMismatch);
        }

        Ok(Self {
            magic,
            version,
            entry_count,
            index_offset,
            total_bytes,
            crc32: stored_crc,
            reserved: [0; 3],
        })
    }
}

/// 16-byte Overlay Directory entry record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(4))]
pub struct OverlayDirectoryEntry {
    pub module_id: u32,
    pub flash_offset: u32,
    pub code_size: u32,
    pub crc32: u32,
}

impl OverlayDirectoryEntry {
    pub const SIZE: usize = core::mem::size_of::<Self>(); // 16 bytes

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.module_id.to_le_bytes());
        buf[4..8].copy_from_slice(&self.flash_offset.to_le_bytes());
        buf[8..12].copy_from_slice(&self.code_size.to_le_bytes());
        buf[12..16].copy_from_slice(&self.crc32.to_le_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let module_id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let flash_offset = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let code_size = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let crc32 = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        Self {
            module_id,
            flash_offset,
            code_size,
            crc32,
        }
    }
}

