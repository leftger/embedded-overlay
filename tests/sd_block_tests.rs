use embedded_overlay::block::{AsyncBlockDevice, BlockDeviceAdapter};
use embedded_overlay::crc::crc32;
use embedded_overlay::header::{OverlayHeader, VfsAssetEntry, VfsSuperblock};
use embedded_overlay::overlay::{OverlayManager, OverlayModule, OverlaySlot};
use embedded_overlay::partition::PartitionView;
use embedded_overlay::vfs::VfsReader;
use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};

use sequential_storage::cache::Cache;
use sequential_storage::map::{MapConfig, MapStorage};

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

/// Simulated 512-byte block SD / microSD card
pub struct MockSdCard<const NUM_BLOCKS: usize> {
    blocks: [[u8; 512]; NUM_BLOCKS],
}

impl<const NUM_BLOCKS: usize> MockSdCard<NUM_BLOCKS> {
    pub fn new() -> Self {
        Self {
            blocks: [[0xFF; 512]; NUM_BLOCKS],
        }
    }
}

impl<const NUM_BLOCKS: usize> AsyncBlockDevice for MockSdCard<NUM_BLOCKS> {
    type Error = ();

    async fn read_blocks(&mut self, start_block: u32, buffer: &mut [u8]) -> Result<(), Self::Error> {
        let count = buffer.len() / 512;
        let start = start_block as usize;
        for i in 0..count {
            buffer[i * 512..(i + 1) * 512].copy_from_slice(&self.blocks[start + i]);
        }
        Ok(())
    }

    async fn write_blocks(&mut self, start_block: u32, buffer: &[u8]) -> Result<(), Self::Error> {
        let count = buffer.len() / 512;
        let start = start_block as usize;
        for i in 0..count {
            self.blocks[start + i].copy_from_slice(&buffer[i * 512..(i + 1) * 512]);
        }
        Ok(())
    }

    fn num_blocks(&self) -> u64 {
        NUM_BLOCKS as u64
    }
}

#[repr(align(16))]
struct AlignedSlot<const N: usize>([u8; N]);

#[repr(C)]
#[derive(Clone, Copy)]
struct MathArgs {
    val: i32,
}

unsafe extern "C" fn square_op(args: MathArgs) -> i32 {
    args.val * args.val
}

struct SquareModule;
unsafe impl OverlayModule for SquareModule {
    const MODULE_ID: u32 = 0x9001;
    type Args = MathArgs;
    type Output = i32;
}

#[test]
fn test_microsd_block_adapter_unaligned_and_aligned() {
    block_on(async {
        let sd = MockSdCard::<64>::new(); // 32 KB SD card
        let mut adapter = BlockDeviceAdapter::<_, 512>::new(sd);

        // Write unaligned 10 bytes at offset 13 (spans within block 0)
        let data1 = [0x42u8; 10];
        adapter.write(13, &data1).await.unwrap();

        let mut read_buf1 = [0u8; 10];
        adapter.read(13, &mut read_buf1).await.unwrap();
        assert_eq!(read_buf1, data1);

        // Write 1024 bytes (exact 2 blocks: blocks 1 and 2)
        let data2 = [0x77u8; 1024];
        adapter.write(512, &data2).await.unwrap();

        let mut read_buf2 = [0u8; 1024];
        adapter.read(512, &mut read_buf2).await.unwrap();
        assert_eq!(read_buf2, data2);

        // Cross-block boundary read (starts at 510, length 6 -> bytes 510..516 spanning block 0 and block 1)
        let mut cross_buf = [0u8; 6];
        adapter.read(510, &mut cross_buf).await.unwrap();
        assert_eq!(&cross_buf[2..], &[0x77; 4]); // first 4 bytes of block 1
    });
}

#[test]
fn test_microsd_overlay_loading_and_execution() {
    block_on(async {
        let sd = MockSdCard::<64>::new();
        let mut adapter = BlockDeviceAdapter::<_, 512>::new(sd);

        // Write overlay module onto microSD card at offset 512 (Block 1)
        let fn_ptr = square_op as *const () as usize;
        let code_bytes = fn_ptr.to_le_bytes();
        let header = OverlayHeader::new(
            SquareModule::MODULE_ID,
            code_bytes.len() as u32,
            0,
            0,
            crc32(&code_bytes),
        );

        adapter.write(512, &header.to_bytes()).await.unwrap();
        adapter.write(512 + OverlayHeader::SIZE as u32, &code_bytes).await.unwrap();

        // Configure RAM overlay slot
        static mut RAM_SLOT: AlignedSlot<64> = AlignedSlot([0; 64]);
        let slots = [OverlaySlot::from_static_slice(unsafe {
            &mut *core::ptr::addr_of_mut!(RAM_SLOT.0)
        })];

        // Mount OverlayManager directly onto microSD card adapter!
        let mut manager = OverlayManager::<_, 1>::new(adapter, slots);

        let slot_idx = manager
            .ensure_resident_typed::<SquareModule>(512)
            .await
            .expect("Load overlay from microSD");
        assert_eq!(slot_idx, 0);
        assert!(manager.is_resident(SquareModule::MODULE_ID));
    });
}

#[test]
fn test_microsd_vfs_streaming_and_partitioning() {
    block_on(async {
        let sd = MockSdCard::<128>::new(); // 64 KB SD card
        let mut adapter = BlockDeviceAdapter::<_, 512>::new(sd);

        // Prepare VFS at offset 1024 (Block 2)
        let asset_data = [0xEEu8; 128];
        let entry = VfsAssetEntry {
            asset_id: 0x8888,
            flash_offset: 0,
            length: 128,
            flags: 0,
            crc16: 0,
        };
        let superblock = VfsSuperblock::new(1, 32, 48, 48 + 128);

        adapter.write(1024, &superblock.to_bytes()).await.unwrap();
        adapter.write(1024 + 32, &entry.to_bytes()).await.unwrap();
        adapter.write(1024 + 48, &asset_data).await.unwrap();

        // Mount VFS from microSD
        let mut vfs = VfsReader::mount(&mut adapter, 1024).await.unwrap();
        let mut stream_buf = [0u8; 64];
        let n = vfs.stream_chunk(0x8888, 10, &mut stream_buf).await.unwrap();
        assert_eq!(n, 64);
        assert_eq!(stream_buf, [0xEE; 64]);

        // Slice an isolated partition for sequential-storage on the microSD card
        let mut part = PartitionView::new(&mut adapter, 16384, 8192);
        let mut data_buf = [0u8; 256];
        let config = MapConfig::new(0..8192);
        let mut map = MapStorage::new(&mut part, config, Cache::new_uncached());

        map.store_item(&mut data_buf, &0x55u16, &12345u32).await.unwrap();
        let val: Option<u32> = map.fetch_item(&mut data_buf, &0x55u16).await.unwrap();
        assert_eq!(val, Some(12345));
    });
}
