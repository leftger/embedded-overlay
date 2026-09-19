use embedded_overlay::header::{VfsAssetEntry, VfsSuperblock};
use embedded_overlay::mock::MockFlash;
use embedded_overlay::vfs::{LruSectorCache, VfsReader};
use embedded_overlay::VfsError;

fn block_on<F: core::future::Future>(mut fut: F) -> F::Output {
    use core::pin::Pin;
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn noop_clone(_: *const ()) -> RawWaker {
        RawWaker::new(core::ptr::null(), &VTABLE)
    }
    fn noop(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(noop_clone, noop, noop, noop);

    let raw_waker = RawWaker::new(core::ptr::null(), &VTABLE);
    let waker = unsafe { Waker::from_raw(raw_waker) };
    let mut cx = Context::from_waker(&waker);

    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(res) => return res,
            Poll::Pending => {}
        }
    }
}

const ASSET_DOOM_TEXTURE: u32 = 0x2001;
const ASSET_FONT_BDF: u32 = 0x2002;
const ASSET_AUDIO_CLIP: u32 = 0x2003;

#[test]
fn test_vfs_mount_and_streaming() {
    block_on(async {
        let mut flash = MockFlash::<65536>::new();

        let asset1_data = [0xAAu8; 128]; // Texture
        let asset2_data = [0xBBu8; 64]; // Font
        let asset3_data = [0xCCu8; 256]; // Audio clip

        // Construct VFS layout in mock flash starting at offset 0
        // Offset 0..32: Superblock
        // Offset 32..80: 3x Asset Entries (16 bytes each)
        // Offset 80..: Contiguous Asset Blobs
        let index_offset = 32u32;
        let data_offset = index_offset + (3 * VfsAssetEntry::SIZE as u32);

        let entry1 = VfsAssetEntry {
            asset_id: ASSET_DOOM_TEXTURE,
            flash_offset: 0,
            length: asset1_data.len() as u32,
            flags: 0,
            crc16: 0,
        };
        let entry2 = VfsAssetEntry {
            asset_id: ASSET_FONT_BDF,
            flash_offset: asset1_data.len() as u32,
            length: asset2_data.len() as u32,
            flags: 0,
            crc16: 0,
        };
        let entry3 = VfsAssetEntry {
            asset_id: ASSET_AUDIO_CLIP,
            flash_offset: (asset1_data.len() + asset2_data.len()) as u32,
            length: asset3_data.len() as u32,
            flags: 0,
            crc16: 0,
        };

        let total_bytes =
            data_offset + (asset1_data.len() + asset2_data.len() + asset3_data.len()) as u32;
        let superblock = VfsSuperblock::new(3, index_offset, data_offset, total_bytes);

        // Write to flash
        flash.data_mut()[0..VfsSuperblock::SIZE].copy_from_slice(&superblock.to_bytes());
        flash.data_mut()[32..48].copy_from_slice(&entry1.to_bytes());
        flash.data_mut()[48..64].copy_from_slice(&entry2.to_bytes());
        flash.data_mut()[64..80].copy_from_slice(&entry3.to_bytes());

        let d_off = data_offset as usize;
        flash.data_mut()[d_off..d_off + 128].copy_from_slice(&asset1_data);
        flash.data_mut()[d_off + 128..d_off + 192].copy_from_slice(&asset2_data);
        flash.data_mut()[d_off + 192..d_off + 448].copy_from_slice(&asset3_data);

        // Mount VFS
        let mut vfs = VfsReader::mount(flash, 0).await.expect("Mount VFS");
        assert_eq!(vfs.asset_count(), 3);

        // Verify asset lengths
        assert_eq!(vfs.asset_len(ASSET_DOOM_TEXTURE).await.unwrap(), 128);
        assert_eq!(vfs.asset_len(ASSET_FONT_BDF).await.unwrap(), 64);
        assert_eq!(vfs.asset_len(ASSET_AUDIO_CLIP).await.unwrap(), 256);

        // Stream complete asset
        let mut read_buf = [0u8; 128];
        let bytes_read = vfs.read_asset(ASSET_DOOM_TEXTURE, &mut read_buf).await.unwrap();
        assert_eq!(bytes_read, 128);
        assert_eq!(read_buf, asset1_data);

        // Stream chunked slices (e.g., streaming scanlines of a texture or audio frames)
        let mut chunk = [0u8; 32];
        let n1 = vfs.stream_chunk(ASSET_AUDIO_CLIP, 0, &mut chunk).await.unwrap();
        assert_eq!(n1, 32);
        assert_eq!(chunk, [0xCC; 32]);

        let n2 = vfs.stream_chunk(ASSET_AUDIO_CLIP, 32, &mut chunk).await.unwrap();
        assert_eq!(n2, 32);
        assert_eq!(chunk, [0xCC; 32]);

        // Beyond EOF returns 0 bytes
        let n_eof = vfs.stream_chunk(ASSET_AUDIO_CLIP, 300, &mut chunk).await.unwrap();
        assert_eq!(n_eof, 0);

        // Query non-existent asset
        let err = vfs.asset_len(0x9999).await;
        match err {
            Err(VfsError::AssetNotFound(0x9999)) => {}
            other => panic!("Expected AssetNotFound, got {:?}", other),
        }
    });
}

#[test]
fn test_vfs_lru_sector_cache() {
    block_on(async {
        let mut flash = MockFlash::<65536, 512>::new(); // 512-byte sectors for test

        // Fill sector 0, 1, 2 with distinct patterns
        flash.data_mut()[0..512].fill(0x11);
        flash.data_mut()[512..1024].fill(0x22);
        flash.data_mut()[1024..1536].fill(0x33);

        // Cache with 2 sector slots of 512 bytes
        let mut cache = LruSectorCache::<2, 512>::new();

        // Load sector 0 -> Cache miss, loads from flash
        let sec0 = cache.get_or_load(&mut flash, 0, 0).await.unwrap();
        assert_eq!(sec0[0], 0x11);

        // Load sector 1 -> Cache miss, loads into slot 1
        let sec1 = cache.get_or_load(&mut flash, 1, 0).await.unwrap();
        assert_eq!(sec1[0], 0x22);

        // Access sector 0 again -> Cache hit!
        assert!(cache.get(0).is_some());
        assert_eq!(cache.get(0).unwrap()[0], 0x11);

        // Load sector 2 -> Cache full, evicts LRU (sector 1)
        let sec2 = cache.get_or_load(&mut flash, 2, 0).await.unwrap();
        assert_eq!(sec2[0], 0x33);

        // Sector 0 was accessed recently, so it should still be in cache
        assert!(cache.get(0).is_some());
        // Sector 1 was evicted
        assert!(cache.get(1).is_none());
    });
}
