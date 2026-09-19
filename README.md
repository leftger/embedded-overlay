# embedded-overlay

A modular, hardware-agnostic `no_std` Rust runtime library for microcontrollers across the **entire ARM Cortex-M family**:
* **Cortex-M0 / Cortex-M0+** (`thumbv6m-none-eabi`, e.g. RP2040, STM32G0, SAMD21, nRF51)
* **Cortex-M3** (`thumbv7m-none-eabi`, e.g. STM32F103)
* **Cortex-M4 / Cortex-M4F** (`thumbv7em-none-eabi` / `thumbv7em-none-eabihf`, e.g. STM32F4, nRF52)
* **Cortex-M7** (`thumbv7em-none-eabihf`, e.g. STM32H7, i.MX RT)
* **Cortex-M23 / Cortex-M33** (`thumbv8m.main-none-eabihf`, e.g. STM32WBA, STM32U5, nRF53)

It allows microcontrollers to break through on-chip flash limits using:

1. **RAM-Paged Code Overlays with Asynchronous DMA Streaming**: Dynamically load and execute Thumb-2 machine code modules in dedicated SRAM slots on-demand while Embassy continues running concurrent tasks (such as Bluetooth LE radio events, sensor polling, or audio pipelines).
2. **Double-Buffered Ping-Pong Execution & Prefetching**: Execute compute kernels in Slot A while asynchronously streaming the next module into Slot B over SPI DMA with zero CPU blocking.
3. **Streamable Virtual Filesystem (VFS)**: Stream large binary assets (textures, game maps, audio frames, neural network weights) directly from external SPI flash into RAM buffers with zero CPU overhead.
4. **Partition Adapter for Ecosystem Crates**: Slice physical external flash into isolated logical partitions, exposing standard [`embedded_storage::nor_flash::NorFlash`](https://docs.rs/embedded-storage) and [`embedded_storage_async::nor_flash::ReadNorFlash`](https://docs.rs/embedded-storage-async) traits to [`sequential-storage`](https://crates.io/crates/sequential-storage) and [`cfg-noodle`](https://crates.io/crates/cfg-noodle) for wear-leveled configuration persistence.

---

## Hardware Independence

`embedded-overlay` contains **zero hardcoded GPIO pins or MCU peripherals**. It operates generically over any storage driver implementing `ReadNorFlash` from `embedded-storage-async`.

Your application initializes the concrete SPI peripheral and DMA channels (e.g., using `embassy_stm32::spi::Spi` with `GPDMA1` and `is25lp128f`) and passes the storage handle to the overlay manager.

---

## Architecture Overview

```
+-------------------------------------------------------------------------+
|                  External SPI NOR Flash (e.g. IS25LP128F, 16 MB)         |
|  +-----------------------+-----------------------+-------------------+  |
|  | Partition 0: Overlays | Partition 1: VFS      | Partition 2:      |  |
|  | (.ovl binary modules) | (Contiguous assets)   | sequential-storage|  |
|  +-----------------------+-----------------------+-------------------+  |
+-------------------------------------------------------------------------+
                                 │ (Async SPI DMA Transfers)
                                 ▼
+-------------------------------------------------------------------------+
|                         Microcontroller SRAM (512 KB)                   |
|  +-----------------------+-----------------------+-------------------+  |
|  | Overlay Slot A (32KB) | Overlay Slot B (32KB) | VFS Cache (16KB)  |  |
|  +-----------------------+-----------------------+-------------------+  |
+-------------------------------------------------------------------------+
                                 │
                                 ▼
               Cortex-M33 CPU Execution (Thumb-2 ISB/DSB)
```

---

## Quickstart

### 1. Defining Overlay Slots & Manager

```rust
use embedded_overlay::overlay::{OverlayManager, OverlayModule, OverlaySlot};

// Define fixed 32 KB execution slots in SRAM
#[repr(align(16))]
struct SlotBuffer([u8; 32 * 1024]);

static mut SLOT_A_BUF: SlotBuffer = SlotBuffer([0; 32 * 1024]);
static mut SLOT_B_BUF: SlotBuffer = SlotBuffer([0; 32 * 1024]);

// Define a type-safe module contract
struct PhysicsEngine;
unsafe impl OverlayModule for PhysicsEngine {
    const MODULE_ID: u32 = 0x1001;
    type Args = PhysicsState;
    type Output = PhysicsState;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // 1. Initialize your board's SPI with DMA and flash driver
    let flash = MySpiFlashDriver::new(...);

    // 2. Configure two RAM overlay slots for ping-pong execution
    let slots = [
        OverlaySlot::from_static_slice(unsafe { &mut SLOT_A_BUF.0 }),
        OverlaySlot::from_static_slice(unsafe { &mut SLOT_B_BUF.0 }),
    ];

    let mut overlay_mgr = OverlayManager::<_, 2>::new(flash, slots);

    // 3. Ensure module is resident and execute!
    let slot_idx = overlay_mgr.ensure_resident_typed::<PhysicsEngine>(0x0000).await.unwrap();
    let updated_state = overlay_mgr.call_typed::<PhysicsEngine>(slot_idx, current_state).unwrap();
}
```

### 2. Double-Buffered Ping-Pong Streaming

```rust
// Run intensive computation in Slot A
let result_a = overlay_mgr.call_typed::<ModuleA>(slot_a, input_a);

// Simultaneously stream Module B into Slot B via Embassy async DMA
let slot_b = overlay_mgr.ensure_resident_typed::<ModuleB>(FLASH_OFFSET_B).await.unwrap();

// Instantaneous switchover to Slot B:
let result_b = overlay_mgr.call_typed::<ModuleB>(slot_b, input_b);
```

### 3. Streamable VFS for Textures, Maps, and Audio

```rust
use embedded_overlay::vfs::VfsReader;

// Mount the VFS partition at offset 2 MB
let mut vfs = VfsReader::mount(flash, 0x0020_0000).await.unwrap();

// Stream a 512-byte scanline directly into the display DMA framebuffer
let mut scanline = [0u8; 512];
let bytes_read = vfs.stream_chunk(ASSET_DOOM_TEXTURE, offset, &mut scanline).await.unwrap();
```

### 4. MicroSD Card Support (SPI / SDIO Block Devices)

Because microSD cards use 512-byte blocks with internal wear-leveling, [`BlockDeviceAdapter`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/block.rs) provides a zero-cost bridge from any [`AsyncBlockDevice`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/block.rs) (like an SPI SD card driver) to [`ReadNorFlash`](https://docs.rs/embedded-storage-async):

```rust
use embedded_overlay::block::BlockDeviceAdapter;

// 1. Initialize your microSD SPI driver (e.g. embedded-sdmmc or raw SPI SD card)
let sd_card = MySpiSdCard::new(spi, cs);

// 2. Wrap it with the 512-byte block adapter
let mut flash_adapter = BlockDeviceAdapter::<_, 512>::new(sd_card);

// 3. Mount overlays and VFS directly off the microSD card!
let mut overlay_mgr = OverlayManager::<_, 2>::new(&mut flash_adapter, slots);
let mut vfs = VfsReader::mount(&mut flash_adapter, 0x0010_0000).await.unwrap();
```

* **Whole-Block Zero-Copy DMA**: Reads and writes aligned to 512 bytes stream directly into RAM over SPI DMA with zero intermediate copies.
* **Sub-Block Bounce Buffer**: Seamlessly handles unaligned reads (such as 32-byte overlay headers or 16-byte VFS index records).

---

### 5. Partitioning for `sequential-storage` & `cfg-noodle`

```rust
use embedded_overlay::partition::PartitionView;
use sequential_storage::map::{MapConfig, MapStorage};
use sequential_storage::cache::Cache;

// Create an isolated 64 KB partition on external flash
let part = PartitionView::new(flash, 0x00E0_0000, 64 * 1024);

// Pass the partition directly to sequential-storage
let config = MapConfig::new(0..64 * 1024);
let mut storage = MapStorage::new(part, config, Cache::new_uncached());
storage.store_item(&mut buf, &KEY_BLE_BOND, &bond_data).await.unwrap();
```

---

## Host Packaging & Dual-Flashing Tooling

### 1. Packaging Modules and Assets (`overlay-packer`)

```bash
# Package a compiled module binary into an .ovl container with CRC32
cargo run --features std --bin overlay-packer -- overlay physics_kernel.bin 0x1001 physics.ovl

# Package an asset directory into a contiguous streamable .vfs image
cargo run --features std --bin overlay-packer -- vfs ./assets ./assets.vfs

# Create a unified dual-flash firmware bundle (.fwbundle)
cargo run --features std --bin overlay-packer -- bundle app.bin external.bin STM32WBA65RI 0x08010000 0x00000000 firmware.fwbundle
```

### 2. Dual-Memory Flashing Script (`flash_dual.py`)

The python tool [`tools/flash_dual.py`](file:///home/usuario/Projects/my-repos/embedded-overlay/tools/flash_dual.py) automates programming both on-chip internal flash (via SWD / `probe-rs`) and external SPI flash:

```bash
# 1. Package unified bundle
./tools/flash_dual.py pack --internal app.bin --external external.bin --out firmware.fwbundle

# 2. Flash on-chip internal flash via SWD
./tools/flash_dual.py flash-internal target/thumbv8m.main-none-eabihf/release/wba65-overlay-demo

# 3. Flash external SPI flash via bootloader transport
./tools/flash_dual.py flash-external --port /dev/ttyACM0 external.bin
```

---

## Dual-Memory Bootloader (`bootloader/`)

The repository includes a bare-metal bootloader located in [`bootloader/`](file:///home/usuario/Projects/my-repos/embedded-overlay/bootloader):
* Resides at `0x0800_0000` (Bank 1 Sector 0..7, 64 KB).
* Leaves `0x0801_0000` – `0x0820_0000` (1.9 MB) for your Embassy application.
* Validates application integrity (Stack Pointer in SRAM range, Reset vector in Flash).
* Relocates Vector Table (`SCB.VTOR`) and switches execution cleanly via `cortex_m::asm::bootstrap`.

---

## Complete STM32WBA65 Embassy Demo (`demos/wba65-overlay-demo/`)

A ready-to-run demo application is available in [`demos/wba65-overlay-demo/`](file:///home/usuario/Projects/my-repos/embedded-overlay/demos/wba65-overlay-demo):
* Sets up dual ping-pong RAM overlay execution slots in SRAM.
* Dynamically streams and executes a native physics calculation module in RAM.
* Concurrently runs Embassy background async tasks (`heartbeat_task`).
* Streams asset chunks using [`VfsReader`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/vfs/mod.rs).
* Persists non-volatile state to an isolated partition using [`sequential-storage`](https://crates.io/crates/sequential-storage).

```bash
cd demos/wba65-overlay-demo
cargo run --release
```

---

## License

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))
