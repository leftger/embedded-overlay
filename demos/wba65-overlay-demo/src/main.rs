//! STM32WBA65RI Dual-Memory & Dynamic Overlay Demonstration.
//!
//! Demonstrates:
//! 1. Allocating dual ping-pong RAM overlay execution slots in SRAM.
//! 2. Dynamic loading and execution of code overlays with architecture memory barriers.
//! 3. Zero-copy asset streaming using VfsReader.
//! 4. Non-volatile partition isolation with sequential-storage.

#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

use embedded_overlay::mock::MockFlash;
use embedded_overlay::overlay::{OverlayManager, OverlayModule, OverlaySlot};
use embedded_overlay::partition::PartitionView;
use embedded_overlay::vfs::VfsReader;
use embedded_overlay::header::{OverlayHeader, VfsAssetEntry, VfsSuperblock};
use embedded_overlay::crc::crc32;

use sequential_storage::cache::Cache;
use sequential_storage::map::{MapConfig, MapStorage};

// Define fixed 16 KB execution slots in SRAM
#[repr(align(16))]
struct SlotBuffer([u8; 16 * 1024]);

static mut SLOT_A_MEM: SlotBuffer = SlotBuffer([0; 16 * 1024]);
static mut SLOT_B_MEM: SlotBuffer = SlotBuffer([0; 16 * 1024]);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicsArgs {
    pub velocity: i32,
    pub delta_time: i32,
}

unsafe extern "C" fn physics_step(args: PhysicsArgs) -> i32 {
    args.velocity + (args.delta_time * 98) / 10
}

struct PhysicsKernel;
unsafe impl OverlayModule for PhysicsKernel {
    const MODULE_ID: u32 = 0x5001;
    type Args = PhysicsArgs;
    type Output = i32;
}

#[embassy_executor::task]
async fn heartbeat_task() {
    let mut ticks = 0u32;
    loop {
        defmt::info!("[Heartbeat] Background task running smoothly, tick={}", ticks);
        ticks += 1;
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    defmt::info!("=== STM32WBA65RI Dual-Memory & Overlay Demo ===");

    // Spawn background task to prove concurrency during DMA/flash operations
    spawner.spawn(heartbeat_task().expect("spawn heartbeat"));

    // 1. Initialize storage (Simulated / Hardware SPI NOR Flash)
    let mut storage = MockFlash::<131072, 4096>::new();

    // Pre-populate an overlay binary module at flash offset 0
    let fn_ptr = physics_step as *const () as usize;
    let code_bytes = fn_ptr.to_le_bytes();
    let header = OverlayHeader::new(
        PhysicsKernel::MODULE_ID,
        code_bytes.len() as u32,
        0,
        0,
        crc32(&code_bytes),
    );
    storage.data_mut()[0..OverlayHeader::SIZE].copy_from_slice(&header.to_bytes());
    storage.data_mut()[OverlayHeader::SIZE..OverlayHeader::SIZE + code_bytes.len()]
        .copy_from_slice(&code_bytes);

    // Pre-populate a streamable VFS asset at flash offset 4096 (4 KB)
    let asset_payload = [0x42u8; 256];
    let entry = VfsAssetEntry {
        asset_id: 0x7001,
        flash_offset: 0,
        length: 256,
        flags: 0,
        crc16: 0,
    };
    let superblock = VfsSuperblock::new(1, 32, 48, 48 + 256);
    storage.data_mut()[4096..4096 + VfsSuperblock::SIZE].copy_from_slice(&superblock.to_bytes());
    storage.data_mut()[4096 + 32..4096 + 48].copy_from_slice(&entry.to_bytes());
    storage.data_mut()[4096 + 48..4096 + 48 + 256].copy_from_slice(&asset_payload);

    // 2. Set up Dual-Slot RAM Overlay Manager
    let slots = [
        OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(SLOT_A_MEM.0) }),
        OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(SLOT_B_MEM.0) }),
    ];

    let mut overlay_mgr = OverlayManager::<_, 2>::new(storage, slots);

    defmt::info!("Step 1: Loading PhysicsKernel overlay into RAM Slot...");
    let slot_idx = overlay_mgr
        .ensure_resident_typed::<PhysicsKernel>(0)
        .await
        .expect("Load overlay module");

    defmt::info!("Overlay loaded in Slot {}. Calling native code...", slot_idx);
    let result = overlay_mgr
        .call_typed::<PhysicsKernel>(slot_idx, PhysicsArgs { velocity: 100, delta_time: 2 })
        .expect("Invoke overlay");
    defmt::info!("Physics output calculated in RAM overlay: {}", result);

    // 3. Test VFS Asset Streaming
    defmt::info!("Step 2: Streaming asset 0x7001 from external flash...");
    // Retrieve storage back from overlay manager to mount VFS
    let storage = overlay_mgr.storage_mut();
    // In real system, storage is shared via an async SPI mutex/device; here we demonstrate direct read
    let mut vfs_buf = [0u8; 64];
    let mut vfs = VfsReader::mount(storage, 4096).await.expect("Mount VFS");
    let read_len = vfs.stream_chunk(0x7001, 0, &mut vfs_buf).await.expect("Stream asset");
    defmt::info!("Streamed {} bytes of asset from external flash! First byte: 0x{:02X}", read_len, vfs_buf[0]);

    // 4. Test PartitionView with sequential-storage
    defmt::info!("Step 3: Storing runtime state with sequential-storage on PartitionView...");
    let storage = vfs.storage_mut();
    let mut part = PartitionView::new(storage, 65536, 16384);
    let mut data_buffer = [0u8; 256];
    let config = MapConfig::new(0..16384);
    let mut map = MapStorage::new(&mut part, config, Cache::new_uncached());

    map.store_item(&mut data_buffer, &0x100u16, &9999u32).await.expect("Store item");
    let fetched = map.fetch_item::<u32>(&mut data_buffer, &0x100u16).await.expect("Fetch item");
    defmt::info!("Retrieved non-volatile value from partition: {:?}", fetched);

    defmt::info!("All demonstrations completed successfully!");

    loop {
        Timer::after(Duration::from_secs(5)).await;
    }
}
