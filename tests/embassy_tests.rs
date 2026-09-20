//! Integration tests for Embassy native integration:
//! SharedStorageBus, EmbassyOverlayEngine, safe slot definitions, and concurrent access.

#![cfg(feature = "embassy")]

use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_overlay::crc::crc32;
use embedded_overlay::embassy::{
    call_and_prefetch, init_slot_from_cell, AlignedSlotBuffer, EmbassyOverlayEngine,
    SharedStorageBus,
};
use embedded_overlay::header::{OverlayHeader, VfsAssetEntry, VfsSuperblock};
use embedded_overlay::mock::MockFlash;
use embedded_overlay::overlay::OverlayModule;
use embedded_overlay::partition::PartitionView;
use embedded_overlay::vfs::VfsReader;
use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};
use static_cell::StaticCell;

fn block_on<F: core::future::Future>(mut fut: F) -> F::Output {
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

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VectorArgs {
    x: i32,
    y: i32,
}

unsafe extern "C" fn vector_add(args: VectorArgs) -> i32 {
    args.x + args.y
}

struct VectorAddKernel;
unsafe impl OverlayModule for VectorAddKernel {
    const MODULE_ID: u32 = 0x8001;
    type Args = VectorArgs;
    type Output = i32;
}

unsafe extern "C" fn vector_mult(args: VectorArgs) -> i32 {
    args.x * args.y
}

struct VectorMultKernel;
unsafe impl OverlayModule for VectorMultKernel {
    const MODULE_ID: u32 = 0x8002;
    type Args = VectorArgs;
    type Output = i32;
}

fn write_overlay_module<const CAPACITY: usize>(
    flash: &mut MockFlash<CAPACITY>,
    offset: u32,
    module_id: u32,
    fn_ptr: usize,
) {
    let code_bytes = fn_ptr.to_le_bytes();
    let header = OverlayHeader::new(
        module_id,
        code_bytes.len() as u32,
        0,
        0,
        crc32(&code_bytes),
    );
    let offset = offset as usize;
    flash.data_mut()[offset..offset + OverlayHeader::SIZE].copy_from_slice(&header.to_bytes());
    flash.data_mut()[offset + OverlayHeader::SIZE..offset + OverlayHeader::SIZE + code_bytes.len()]
        .copy_from_slice(&code_bytes);
}

#[test]
fn test_embassy_shared_storage_bus_concurrency() {
    block_on(async {
        let mut flash = MockFlash::<131072>::new();

        // 1. Write an overlay module at offset 0
        write_overlay_module(
            &mut flash,
            0,
            VectorAddKernel::MODULE_ID,
            vector_add as *const () as usize,
        );

        // 2. Write a VFS image at offset 4096 (4 KB)
        let asset_payload = [0x55u8; 128];
        let entry = VfsAssetEntry {
            asset_id: 0x9001,
            flash_offset: 0,
            length: 128,
            flags: 0,
            crc16: 0,
        };
        let superblock = VfsSuperblock::new(1, 32, 48, 48 + 128);
        flash.data_mut()[4096..4096 + VfsSuperblock::SIZE].copy_from_slice(&superblock.to_bytes());
        flash.data_mut()[4096 + 32..4096 + 48].copy_from_slice(&entry.to_bytes());
        flash.data_mut()[4096 + 48..4096 + 48 + 128].copy_from_slice(&asset_payload);

        // Wrap flash in the shared bus
        let bus = SharedStorageBus::<CriticalSectionRawMutex, _>::new(flash);

        // Create execution slots using safe static cells with 16-byte alignment!
        static CELL_MEM: StaticCell<AlignedSlotBuffer<1024>> = StaticCell::new();
        let slots = [init_slot_from_cell(&CELL_MEM)];

        // All three subsystems take handles to the exact same bus!
        let mut overlay_mgr = embedded_overlay::OverlayManager::new(bus.handle(), slots);
        let mut vfs = VfsReader::mount(bus.handle(), 4096).await.unwrap();
        let mut partition = PartitionView::new(bus.handle(), 65536, 16384);

        // Interleaved operations to prove concurrent sharing without ownership passing:
        // a. Load and execute overlay
        let slot = overlay_mgr.ensure_resident_typed::<VectorAddKernel>(0).await.unwrap();
        let sum = overlay_mgr.call_typed::<VectorAddKernel>(slot, VectorArgs { x: 15, y: 25 }).unwrap();
        assert_eq!(sum, 40);

        // b. Stream VFS asset
        let mut vfs_buf = [0u8; 64];
        let n = vfs.stream_chunk(0x9001, 0, &mut vfs_buf).await.unwrap();
        assert_eq!(n, 64);
        assert_eq!(vfs_buf[0], 0x55);

        // c. Write and read from isolated partition
        partition.erase(0, 4096).await.unwrap();
        partition.write(0, &[1, 2, 3, 4]).await.unwrap();
        let mut part_read = [0u8; 4];
        partition.read(0, &mut part_read).await.unwrap();
        assert_eq!(part_read, [1, 2, 3, 4]);

        // d. Re-call overlay (still resident and accessible!)
        let sum2 = overlay_mgr.call_typed::<VectorAddKernel>(slot, VectorArgs { x: 100, y: 200 }).unwrap();
        assert_eq!(sum2, 300);
    });
}

#[test]
fn test_embassy_overlay_engine_transparent_dispatch() {
    block_on(async {
        let mut flash = MockFlash::<131072>::new();

        // Write two modules
        write_overlay_module(
            &mut flash,
            0x0000,
            VectorAddKernel::MODULE_ID,
            vector_add as *const () as usize,
        );
        write_overlay_module(
            &mut flash,
            0x1000,
            VectorMultKernel::MODULE_ID,
            vector_mult as *const () as usize,
        );

        // Safe static cell allocations with guaranteed 16-byte alignment
        static CELL_A: StaticCell<AlignedSlotBuffer<1024>> = StaticCell::new();
        static CELL_B: StaticCell<AlignedSlotBuffer<1024>> = StaticCell::new();
        let slots = [init_slot_from_cell(&CELL_A), init_slot_from_cell(&CELL_B)];

        let mut engine = EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 2>::new(flash, slots);

        // Register modules once in registry:
        engine.register_module(VectorAddKernel::MODULE_ID, 0x0000).unwrap();
        engine.register_module(VectorMultKernel::MODULE_ID, 0x1000).unwrap();

        // Transparent call: no slot index, no flash offset!
        let sum = engine.call::<VectorAddKernel>(VectorArgs { x: 10, y: 20 }).await.unwrap();
        assert_eq!(sum, 30);
        assert!(engine.is_resident::<VectorAddKernel>().await);

        // Transparent call to second module:
        let product = engine.call::<VectorMultKernel>(VectorArgs { x: 5, y: 6 }).await.unwrap();
        assert_eq!(product, 30);
        assert!(engine.is_resident::<VectorMultKernel>().await);

        // Pipelining / prefetch helper:
        let _sum_pipelined = call_and_prefetch::<VectorAddKernel, VectorMultKernel, _, _, 2, 16, _>(
            &engine,
            VectorArgs { x: 50, y: 50 },
        )
        .await
        .unwrap();
        assert!(engine.is_resident::<VectorMultKernel>().await);
    });
}

// 3. Test transparent memory_overlay! macro with AUTOMATIC compile-time ID derivation
embedded_overlay::memory_overlay! {
    pub async fn compute_accel(velocity: i32, time_delta: i32) -> i32 {
        velocity * time_delta + 42
    }
}

#[test]
fn test_memory_overlay_macro_transparency() {
    block_on(async {
        // Verify compile-time automatic ID was derived from function name:
        assert_eq!(
            compute_accel::Module::MODULE_ID,
            embedded_overlay::fnv1a_hash("compute_accel")
        );

        let mut flash = MockFlash::<65536>::new();
        write_overlay_module(
            &mut flash,
            0x0000,
            compute_accel::Module::MODULE_ID,
            compute_accel::entry_point as *const () as usize,
        );

        static CELL: StaticCell<AlignedSlotBuffer<1024>> = StaticCell::new();
        let slots = [init_slot_from_cell(&CELL)];
        let mut engine = EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 1>::new(flash, slots);
        engine.register_module(compute_accel::Module::MODULE_ID, 0x0000).unwrap();

        // Direct call to macro-generated function!
        // No struct packing, no manual module ID, no slot index!
        let result = compute_accel(&engine, 10, 5).await.unwrap();
        assert_eq!(result, 10 * 5 + 42);
    });
}

// 4. Test automated engine.mount() with OverlayDirectory table & sequential discovery
#[test]
fn test_engine_auto_mount_directory_and_sequential() {
    use embedded_overlay::header::{OverlayDirectory, OverlayDirectoryEntry};

    block_on(async {
        // Test Mode A: OverlayDirectory table
        let mut flash = MockFlash::<65536>::new();

        let dir = OverlayDirectory::new(2, 32, 32 + 32 + 1024);
        flash.data_mut()[0..32].copy_from_slice(&dir.to_bytes());

        let entry1 = OverlayDirectoryEntry {
            module_id: 0x8001,
            flash_offset: 128,
            code_size: 8,
            crc32: 0,
        };
        let entry2 = OverlayDirectoryEntry {
            module_id: 0x8002,
            flash_offset: 256,
            code_size: 8,
            crc32: 0,
        };
        flash.data_mut()[32..48].copy_from_slice(&entry1.to_bytes());
        flash.data_mut()[48..64].copy_from_slice(&entry2.to_bytes());

        write_overlay_module(&mut flash, 128, 0x8001, vector_add as *const () as usize);
        write_overlay_module(&mut flash, 256, 0x8002, vector_mult as *const () as usize);

        static CELL_A: StaticCell<AlignedSlotBuffer<1024>> = StaticCell::new();
        let slots = [init_slot_from_cell(&CELL_A)];
        let mut engine = EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 1>::new(flash, slots);

        // Mount discovers all modules automatically without any manual registration!
        let count = engine.mount(0).await.expect("Auto-mount directory");
        assert_eq!(count, 2);
        assert_eq!(engine.find_offset(0x8001), Some(128));
        assert_eq!(engine.find_offset(0x8002), Some(256));

        let res = engine.call::<VectorAddKernel>(VectorArgs { x: 7, y: 8 }).await.unwrap();
        assert_eq!(res, 15);

        // Test Mode B: Sequential OVL1 containers
        let mut flash_seq = MockFlash::<65536>::new();
        write_overlay_module(&mut flash_seq, 0, 0x8001, vector_add as *const () as usize);
        // OverlayHeader (32) + usize (8) = 40 bytes
        write_overlay_module(&mut flash_seq, 40, 0x8002, vector_mult as *const () as usize);

        static CELL_B: StaticCell<AlignedSlotBuffer<1024>> = StaticCell::new();
        let slots_b = [init_slot_from_cell(&CELL_B)];
        let mut engine_seq = EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 1>::new(flash_seq, slots_b);

        let count_seq = engine_seq.mount(0).await.expect("Auto-mount sequential");
        assert_eq!(count_seq, 2);
        assert_eq!(engine_seq.find_offset(0x8001), Some(0));
        assert_eq!(engine_seq.find_offset(0x8002), Some(40));

        let res_mult = engine_seq.call::<VectorMultKernel>(VectorArgs { x: 6, y: 7 }).await.unwrap();
        assert_eq!(res_mult, 42);
    });
}

