//! Dual-Flash Firmware Bundle specification and parser.

use crate::crc::crc32;
use crate::error::OverlayError;

/// Magic bytes for dual-flash firmware bundles: "DFW1".
pub const BUNDLE_MAGIC: [u8; 4] = *b"DFW1";
/// Format version for dual-flash firmware bundles.
pub const BUNDLE_VERSION: u16 = 1;

/// 64-byte Header describing a unified dual-flash firmware package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(4))]
pub struct BundleHeader {
    /// Magic signature: `b"DFW1"`.
    pub magic: [u8; 4],
    /// Format version (1).
    pub version: u16,
    /// Header flags (bit 0: compressed, etc.).
    pub flags: u16,
    /// Target chip identifier string (UTF-8, null-padded).
    pub target_chip: [u8; 16],
    /// Target address in on-chip internal flash (e.g. 0x0801_0000).
    pub internal_target_addr: u32,
    /// Length of on-chip internal flash binary in bytes.
    pub internal_size: u32,
    /// IEEE 802.3 CRC32 of internal flash binary.
    pub internal_crc32: u32,
    /// Target address in external SPI flash (e.g. 0x0000_0000).
    pub external_target_addr: u32,
    /// Length of external SPI flash payload (overlays + VFS assets).
    pub external_size: u32,
    /// IEEE 802.3 CRC32 of external SPI flash payload.
    pub external_crc32: u32,
    /// CRC32 of this header (first 60 bytes).
    pub header_crc32: u32,
}

impl BundleHeader {
    pub const SIZE: usize = core::mem::size_of::<Self>(); // 64 bytes

    /// Creates a new dual-flash bundle header with computed CRC.
    pub fn new(
        target_chip: &str,
        internal_target_addr: u32,
        internal_size: u32,
        internal_crc32: u32,
        external_target_addr: u32,
        external_size: u32,
        external_crc32: u32,
    ) -> Self {
        let mut chip_bytes = [0u8; 16];
        let copy_len = target_chip.len().min(16);
        chip_bytes[..copy_len].copy_from_slice(&target_chip.as_bytes()[..copy_len]);

        let mut header = Self {
            magic: BUNDLE_MAGIC,
            version: BUNDLE_VERSION,
            flags: 0,
            target_chip: chip_bytes,
            internal_target_addr,
            internal_size,
            internal_crc32,
            external_target_addr,
            external_size,
            external_crc32,
            header_crc32: 0,
        };

        let bytes = header.to_bytes();
        header.header_crc32 = crc32(&bytes[..60]);
        header
    }

    /// Serializes the header into a 64-byte array.
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..24].copy_from_slice(&self.target_chip);
        buf[24..28].copy_from_slice(&self.internal_target_addr.to_le_bytes());
        buf[28..32].copy_from_slice(&self.internal_size.to_le_bytes());
        buf[32..36].copy_from_slice(&self.internal_crc32.to_le_bytes());
        buf[36..40].copy_from_slice(&self.external_target_addr.to_le_bytes());
        buf[40..44].copy_from_slice(&self.external_size.to_le_bytes());
        buf[44..48].copy_from_slice(&self.external_crc32.to_le_bytes());
        buf[48..52].copy_from_slice(&self.header_crc32.to_le_bytes());
        buf
    }

    /// Deserializes and validates the header from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OverlayError> {
        if bytes.len() < Self::SIZE {
            return Err(OverlayError::Storage);
        }
        let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != BUNDLE_MAGIC {
            return Err(OverlayError::InvalidMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != BUNDLE_VERSION {
            return Err(OverlayError::UnsupportedVersion);
        }
        let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        let mut target_chip = [0u8; 16];
        target_chip.copy_from_slice(&bytes[8..24]);

        let internal_target_addr = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        let internal_size = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
        let internal_crc32 = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);

        let external_target_addr = u32::from_le_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]);
        let external_size = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        let external_crc32 = u32::from_le_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]);

        let header_crc32 = u32::from_le_bytes([bytes[48], bytes[49], bytes[50], bytes[51]]);

        let calculated_crc = crc32(&bytes[..60]);
        if calculated_crc != header_crc32 {
            return Err(OverlayError::HeaderChecksumMismatch);
        }

        Ok(Self {
            magic,
            version,
            flags,
            target_chip,
            internal_target_addr,
            internal_size,
            internal_crc32,
            external_target_addr,
            external_size,
            external_crc32,
            header_crc32,
        })
    }
}
