//! Zero-dependency IEEE 802.3 CRC32 implementation for no_std environments.

/// IEEE 802.3 standard CRC32 polynomial (reversed: 0xEDB88320).
const POLY: u32 = 0xEDB88320;

/// Generate CRC32 table at compile time.
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLY;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = make_table();

/// Computes the IEEE 802.3 CRC32 checksum of a byte slice.
#[inline]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let index = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC_TABLE[index];
    }
    !crc
}

/// Computes a 32-bit FNV-1a hash of a string at compile time.
///
/// Used for automatic, collision-resistant module ID generation from function names.
pub const fn fnv1a_hash(s: &str) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        i += 1;
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_standard() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(crc32(b""), 0x00000000);
    }

    #[test]
    fn test_fnv1a_hash() {
        assert_ne!(fnv1a_hash("compute_physics"), 0);
        assert_ne!(fnv1a_hash("compute_physics"), fnv1a_hash("render_frame"));
        // Deterministic check
        assert_eq!(fnv1a_hash("test"), 0xAFD0_71E5);
    }
}
