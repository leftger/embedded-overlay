use embedded_overlay::mock::MockFlash;
use embedded_overlay::partition::{PartitionError, PartitionView};
use embedded_storage_async::nor_flash::{
    NorFlash as AsyncNorFlash, ReadNorFlash as AsyncReadNorFlash,
};
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

#[test]
fn test_partition_view_bounds_and_isolation() {
    block_on(async {
        let flash = MockFlash::<65536, 4096>::new();

        // Create an 8 KB partition located at offset 4096..12288
        let mut part = PartitionView::new(flash, 4096, 8192);

        assert_eq!(part.base_offset(), 4096);
        assert_eq!(part.size(), 8192);
        assert_eq!(part.capacity(), 8192);

        // Write within partition at relative offset 0 -> maps to physical 4096
        let write_data = [0x55u8; 16];
        part.write(0, &write_data).await.expect("Write relative 0");

        // Read back from partition
        let mut read_buf = [0u8; 16];
        part.read(0, &mut read_buf).await.expect("Read relative 0");
        assert_eq!(read_buf, write_data);

        // Verify that physical flash before partition (0..4096) is completely untouched
        let raw_flash = part.inner();
        assert_eq!(&raw_flash.data()[0..16], &[0xFF; 16]);
        // Verify that physical flash at 4096 contains the written data
        assert_eq!(&raw_flash.data()[4096..4112], &write_data);

        // Attempting to read/write out of bounds returns PartitionError::OutOfBounds
        let mut oob_buf = [0u8; 32];
        let err = part.read(8180, &mut oob_buf).await; // 8180 + 32 = 8212 > 8192
        match err {
            Err(PartitionError::OutOfBounds) => {}
            other => panic!("Expected OutOfBounds, got {:?}", other),
        }

        let err_write = part.write(8180, &oob_buf).await;
        match err_write {
            Err(PartitionError::OutOfBounds) => {}
            other => panic!("Expected OutOfBounds, got {:?}", other),
        }
    });
}

#[test]
fn test_sequential_storage_on_partition() {
    block_on(async {
        let flash = MockFlash::<65536, 4096>::new();

        // Dedicated 16 KB partition at 8192..24576 (4 pages of 4096 bytes)
        let part = PartitionView::new(flash, 8192, 16384);

        // Run sequential-storage map within the 0..16384 relative range
        let mut data_buffer = [0u8; 256];
        let config = MapConfig::new(0..16384);
        let mut map = MapStorage::new(part, config, Cache::new_uncached());

        // Store item with key 0x42
        map.store_item(&mut data_buffer, &0x42u16, &104729u32)
            .await
            .expect("sequential-storage store_item");

        // Fetch item back
        let fetched: Option<u32> = map
            .fetch_item::<u32>(&mut data_buffer, &0x42u16)
            .await
            .expect("sequential-storage fetch_item");

        assert_eq!(fetched, Some(104729));

        // Verify that outside the partition (0..8192 and 24576..65536) remained 0xFF
        let underlying = map.destroy().0.into_inner();
        assert_eq!(&underlying.data()[0..128], &[0xFF; 128]);
        assert_eq!(&underlying.data()[24576..24704], &[0xFF; 128]);
    });
}
