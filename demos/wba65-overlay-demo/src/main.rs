//! STM32WBA65RI Dual-Memory & Dynamic Overlay Demonstration.
//!
//! Demonstrates:
//! 1. Allocating dual ping-pong RAM overlay execution slots in SRAM.
//! 2. Dynamic loading and execution of code overlays with ICACHE-aware instruction
//!    synchronization (see `IcacheCodeSync`).
//! 3. Zero-copy asset streaming using VfsReader.
//! 4. Non-volatile partition isolation with sequential-storage.

#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;
use embassy_stm32 as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

use embassy_stm32::icache::{Icache, OutputBurst, RegionConfig, RegionSize, TrafficRoute};

use embedded_overlay::embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_overlay::crc::crc32;
use embedded_overlay::embassy::{EmbassyOverlayEngine, SharedStorageBus};
use embedded_overlay::header::{OverlayHeader, VfsAssetEntry, VfsSuperblock};
use embedded_overlay::mock::MockFlash;
use embedded_overlay::overlay::OverlaySlot;
use embedded_overlay::partition::PartitionView;
use embedded_overlay::vfs::VfsReader;
use embedded_overlay::InstructionCacheSync;

use sequential_storage::cache::Cache;
use sequential_storage::map::{MapConfig, MapStorage};

// Define fixed 16 KB execution slots in SRAM
#[repr(align(16))]
struct SlotBuffer([u8; 16 * 1024]);

static mut SLOT_A_MEM: SlotBuffer = SlotBuffer([0; 16 * 1024]);
static mut SLOT_B_MEM: SlotBuffer = SlotBuffer([0; 16 * 1024]);

/// Physical base of the SRAM1/SRAM2 region on STM32WBA6xxI: 448 KB SRAM1 + 64 KB SRAM2,
/// contiguous at 0x2000_0000..0x2007_FFFF (RM0515 section 2.3).
const SRAM_PHYS_BASE: u32 = 0x2000_0000;

/// 2 MB-aligned code-region alias window that the ICACHE remaps onto [`SRAM_PHYS_BASE`].
///
/// RM0515 section 8.4.7 uses exactly this pairing (SRAM [0x2000_0000:0x201F_FFFF] remapped into
/// code [0x1000_0000:0x101F_FFFF]); section 8.4.6 also names 0x0A00_0000-0x0AFF_FFFF and
/// 0x0E00_0000-0x0EFF_FFFF. Whatever base is chosen, it must be 2 MB aligned and inside the
/// code region. Verify the final choice against the device on the bench.
const SRAM_CODE_ALIAS: u32 = 0x1000_0000;

/// Bridges `embassy_stm32`'s ICACHE driver into the hardware-agnostic overlay engine.
///
/// The register-level cache driver belongs to the HAL; `embedded-overlay` only defines *when*
/// instruction fetch must be synchronized, and this is that "when".
struct IcacheCodeSync {
    icache: Icache<'static>,
}

impl InstructionCacheSync for IcacheCodeSync {
    fn code_loaded(&mut self) {
        // 1. Commit the stores that populated the slot.
        cortex_m::asm::dsb();
        // 2. Invalidate the ICACHE. `DSB`/`ISB` alone do NOT invalidate the hardware cache, so
        //    without this an overlay reload can execute stale instructions.
        self.icache.invalidate();
        // 3. Flush the pipeline so the next fetch observes the new code.
        cortex_m::asm::isb();
    }
}

// Define overlay function using memory_overlay! macro:
// The compiler automatically hashes "compute_physics" to derive its 32-bit module ID,
// generates the C-ABI entry point, and links it into section .overlay.compute_physics!
embedded_overlay::memory_overlay! {
    pub async fn compute_physics(velocity: i32, delta_time: i32) -> i32 {
        velocity + (delta_time * 98) / 10
    }
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
    let p = embassy_stm32::init(Default::default());
    defmt::info!("=== STM32WBA65RI Dual-Memory & Overlay Demo ===");

    // Spawn background task to prove concurrency during DMA/flash operations
    spawner.spawn(heartbeat_task().expect("spawn heartbeat"));

    // Configure the ICACHE remap window before enabling the cache: the ICACHE_CRRx registers are
    // write-locked while EN = 1, so the cache must be disabled here. Both 16 KB slots live inside
    // the first 2 MB of SRAM, so a single 2 MB remap region covers them.
    let mut icache = Icache::new(p.ICACHE);
    icache.disable();
    icache
        .enable_remap_region(
            0,
            RegionConfig {
                // Incoming code-region alias address -> physical SRAM address.
                base_address: SRAM_CODE_ALIAS,
                remap_address: SRAM_PHYS_BASE,
                size: RegionSize::Mb2,
                // Route remapped SRAM traffic over master 2 (slow bus) so flash fetches on
                // master 1 are not stalled by SRAM line refills.
                route: TrafficRoute::Master2,
                burst: OutputBurst::Wrap,
            },
        )
        .expect("configure ICACHE remap region 0");
    icache.enable();

    // 1. Initialize storage (Simulated / Hardware SPI NOR Flash)
    let mut storage = MockFlash::<131072, 4096>::new();

    // Pre-populate overlay binary module at flash offset 0
    let fn_ptr = compute_physics::entry_point as *const () as usize;
    let code_bytes = fn_ptr.to_le_bytes();
    let header = OverlayHeader::new(
        compute_physics::Module::MODULE_ID,
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

    // Wrap storage in SharedStorageBus so overlays, VFS, and sequential-storage share it concurrently!
    let bus = SharedStorageBus::<CriticalSectionRawMutex, _>::new(storage);

    // 2. Set up Dual-Slot RAM Overlay Engine
    //
    // Code is written to the physical SRAM address (`ram_addr`) over the system bus, but executed
    // through the ICACHE alias (`exec_addr`), so instruction fetch goes through the cache. The
    // `IcacheCodeSync` hook invalidates the cache after each load.
    let mut slots = [
        OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(SLOT_A_MEM.0) }),
        OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(SLOT_B_MEM.0) }),
    ];
    for slot in slots.iter_mut() {
        slot.set_exec_addr(SRAM_CODE_ALIAS as usize + (slot.ram_addr - SRAM_PHYS_BASE as usize));
    }

    let mut engine =
        EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 2, 16, IcacheCodeSync>::with_sync(
            bus.handle(),
            slots,
            IcacheCodeSync { icache },
        );

    defmt::info!("Step 1: Auto-mounting external storage overlays...");
    let count = engine.mount(0).await.expect("Mount overlay partition");
    defmt::info!("Auto-discovered and registered {} overlay module(s)!", count);

    defmt::info!("Step 2: Calling compute_physics transparently...");
    let result = compute_physics(&engine, 100, 2).await.expect("Invoke compute_physics overlay");
    defmt::info!("Physics output calculated in RAM overlay: {}", result);

    // 3. Test VFS Asset Streaming concurrently on shared bus
    defmt::info!("Step 3: Streaming asset 0x7001 from external flash...");
    let mut vfs_buf = [0u8; 64];
    let mut vfs = VfsReader::mount(bus.handle(), 4096).await.expect("Mount VFS");
    let read_len = vfs.stream_chunk(0x7001, 0, &mut vfs_buf).await.expect("Stream asset");
    defmt::info!(
        "Streamed {} bytes of asset from external flash! First byte: 0x{:02X}",
        read_len,
        vfs_buf[0]
    );

    // 4. Test PartitionView with sequential-storage concurrently on shared bus
    defmt::info!("Step 4: Storing runtime state with sequential-storage on PartitionView...");
    let mut part = PartitionView::new(bus.handle(), 65536, 16384);
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
