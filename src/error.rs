//! Error definitions for embedded-overlay and streamable VFS.

use core::fmt;

/// Errors that can occur during overlay loading, verification, and dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OverlayError {
    /// Underlying storage read error.
    Storage,
    /// Header magic does not match b"OVL1".
    InvalidMagic,
    /// Header version is unsupported.
    UnsupportedVersion,
    /// Header CRC mismatch.
    HeaderChecksumMismatch,
    /// The machine code payload CRC32 does not match the header expectation.
    PayloadCrcMismatch { expected: u32, calculated: u32 },
    /// The module's machine code size exceeds the capacity of the target RAM slot.
    SlotTooSmall { required: usize, available: usize },
    /// The specified module ID was not found in the overlay table.
    ModuleNotFound(u32),
    /// Invalid alignment for RAM address or entry point.
    InvalidAlignment,
    /// Target slot index is out of range.
    InvalidSlotIndex,
}

impl fmt::Display for OverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage => write!(f, "Storage read error"),
            Self::InvalidMagic => write!(f, "Invalid overlay header magic"),
            Self::UnsupportedVersion => write!(f, "Unsupported overlay format version"),
            Self::HeaderChecksumMismatch => write!(f, "Overlay header checksum mismatch"),
            Self::PayloadCrcMismatch { expected, calculated } => {
                write!(
                    f,
                    "Payload CRC32 mismatch: expected 0x{expected:08X}, calculated 0x{calculated:08X}"
                )
            }
            Self::SlotTooSmall { required, available } => {
                write!(
                    f,
                    "RAM slot too small: required {required} bytes, available {available} bytes"
                )
            }
            Self::ModuleNotFound(id) => write!(f, "Overlay module ID {id} not found"),
            Self::InvalidAlignment => write!(f, "Invalid memory or entry point alignment"),
            Self::InvalidSlotIndex => write!(f, "Overlay slot index out of bounds"),
        }
    }
}

/// Errors that can occur during VFS asset lookup and streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VfsError {
    /// Underlying storage read error.
    Storage,
    /// Superblock magic does not match b"VFS1".
    InvalidMagic,
    /// Superblock version unsupported.
    UnsupportedVersion,
    /// Superblock CRC32 mismatch.
    SuperblockCrcMismatch,
    /// Asset ID not found in VFS index.
    AssetNotFound(u32),
    /// Provided buffer is too small to receive the requested chunk.
    BufferTooSmall { required: usize, provided: usize },
    /// Read offset is beyond the asset boundary.
    OffsetOutOfBounds,
    /// Asset CRC16 or CRC32 integrity check failed.
    IntegrityCheckFailed,
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage => write!(f, "Storage read error"),
            Self::InvalidMagic => write!(f, "Invalid VFS superblock magic"),
            Self::UnsupportedVersion => write!(f, "Unsupported VFS version"),
            Self::SuperblockCrcMismatch => write!(f, "VFS superblock CRC32 mismatch"),
            Self::AssetNotFound(id) => write!(f, "Asset ID {id} not found in VFS index"),
            Self::BufferTooSmall { required, provided } => {
                write!(
                    f,
                    "Buffer too small: required {required} bytes, provided {provided} bytes"
                )
            }
            Self::OffsetOutOfBounds => write!(f, "Asset read offset out of bounds"),
            Self::IntegrityCheckFailed => write!(f, "Asset data integrity check failed"),
        }
    }
}
