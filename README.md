# embedded-overlay

<p align="center">
  <img src="assets/aztec_rustacean.png" alt="embedded-overlay" width="100%">
</p>

[![crates.io](https://img.shields.io/crates/v/embedded-overlay.svg)](https://crates.io/crates/embedded-overlay)
[![docs.rs](https://img.shields.io/docsrs/embedded-overlay)](https://docs.rs/embedded-overlay)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

A modular, hardware-agnostic `no_std` Rust runtime and tooling ecosystem for microcontrollers across the **entire ARM Cortex-M family**:
* **Cortex-M0 / Cortex-M0+** (`thumbv6m-none-eabi`, e.g. RP2040, STM32G0, SAMD21, nRF51)
* **Cortex-M3** (`thumbv7m-none-eabi`, e.g. STM32F103)
* **Cortex-M4 / Cortex-M4F** (`thumbv7em-none-eabi` / `thumbv7em-none-eabihf`, e.g. STM32F4, nRF52)
* **Cortex-M7** (`thumbv7em-none-eabihf`, e.g. STM32H7, i.MX RT)
* **Cortex-M23 / Cortex-M33** (`thumbv8m.main-none-eabihf`, e.g. STM32WBA, STM32U5, nRF53)

`embedded-overlay` breaks through microcontroller on-chip flash limits by turning external SPI NOR flash and MicroSD cards into an expanded executable and filesystem area — **with zero manual packaging, zero manual memory offsets, and zero manual slot management**. Building and running is as simple as:

```bash
cargo run
```

---

## What Problem Does This Solve?

Microcontroller applications increasingly hit the **on-chip memory wall**: modern Bluetooth LE audio stacks, machine learning inference kernels, UI display assets, and telemetry buffers quickly exhaust internal flash (typically 512 KB – 2 MB).

Historically, dynamic overlays were notoriously clunky and error-prone:
* Developers had to manually slice code sections using `objcopy`.
* Developers had to define and synchronize magic numeric module IDs across tools.
* Developers had to calculate and hardcode byte offsets in external flash.
* Developers had to manually manage SRAM buffers, alignment, and generation counters.
* Running required multi-step scripts to package `.ovl` files before flashing.

**`embedded-overlay` completely eliminates this friction**:
1. **Zero Manual IDs**: The [`memory_overlay!`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/macros.rs) macro derives a 32-bit FNV-1a hash from the function name at compile time.
2. **Zero Manual Packaging**: The compiler places overlay code in `.overlay.<fn_name>`. Running `cargo run` automatically extracts machine code, generates `.ovl` containers, and builds external flash images.
3. **Zero Hardcoded Offsets**: On boot, [`engine.mount(0).await`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/engine.rs#L49-L135) reads the [`OverlayDirectory`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/header.rs#L254) table and auto-registers every module.
4. **Transparent Async Invocation**: Call overlay functions like regular async Rust functions: `compute_physics(&engine, 100, 2).await`.
5. **Shared Bus Concurrency**: [`SharedStorageBus`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/shared_storage.rs) allows overlays, VFS asset streaming, and non-volatile key-value persistence ([`sequential-storage`](https://crates.io/crates/sequential-storage)) to share the physical SPI bus concurrently without ownership conflicts.

---

## Architectural Workflow

### 1. The Zero-Touch `cargo run` Pipeline

```
                     Developer runs `cargo run`
                                │
                                ▼
┌──────────────────────────────────────────────────────────────┐
│  rustc / cargo build                                         │
│  - Places functions in #[link_section = ".overlay.<name>"]   │
│  - Compile-time 32-bit FNV-1a ID derivation                  │
│  - Preserves symbols against --gc-sections via #[used]       │
└───────────────────────────────┬──────────────────────────────┘
                                │ passes ELF path to runner
                                ▼
┌──────────────────────────────────────────────────────────────┐
│  tools/flash_dual.py auto-run (or overlay-packer auto-pack)  │
│  1. Discovers all `.overlay.*` sections via readelf          │
│  2. Extracts raw Thumb-2 machine code via llvm-objcopy       │
│  3. Wraps each into .ovl with CRC32 & FNV-1a module ID       │
│  4. Builds `ext_flash.bin` prefixed with `OverlayDirectory`  │
│  5. Builds unified `firmware.fwbundle` container             │
│  6. Executes `probe-rs run` to program and stream defmt RTT  │
└───────────────────────────────┬──────────────────────────────┘
                                │ SWD download & reset
                                ▼
┌──────────────────────────────────────────────────────────────┐
│  Microcontroller Execution (STM32 / Cortex-M)                │
│  1. bus = SharedStorageBus::new(flash_driver);               │
│  2. engine.mount(0).await;       // Auto-discovers all ovls  │
│  3. compute_physics(&engine, 100, 2).await;  // Seamless!    │
└──────────────────────────────────────────────────────────────┘
```

---

### 2. External Storage Binary Layout

External SPI NOR flash or MicroSD cards are formatted into clean, isolated regions:

```
+─────────────────────────────────────────────────────────────────────────+
|                  External SPI NOR Flash or MicroSD Card                 |
+───────────────────────────────────┬───────────────────┬─────────────────+
| Offset 0x0000_0000                | Offset 0x0020_0000| Offset 0x00E0...|
| Partition 0: Code Overlays        | Partition 1: VFS  | Partition 2:    |
|                                   |                   | sequential-     |
| ┌───────────────────────────────┐ | ┌───────────────┐ | storage         |
| │ OverlayDirectory (32B "OVLD") │ | │ VfsSuperblock │ |                 |
| ├───────────────────────────────┤ | │ (32B "VFS1")  │ | ┌─────────────┐ |
| │ Directory Entries (16B each)  │ | ├───────────────┤ | │ MapStorage  │ |
| ├───────────────────────────────┤ | │ VfsAssetEntry │ | │ (BLE bonds, │ |
| │ .ovl Container 1 (Header+Code)│ | ├───────────────┤ | │ calibration,│ |
| ├───────────────────────────────┤ | │ Raw Assets    │ | │ wifi creds) │ |
| │ .ovl Container 2 (Header+Code)│ | │ (textures,    │ | └─────────────┘ |
| └───────────────────────────────┘ | │  audio frames)│ |                 |
|                                   | └───────────────┘ |                 |
+───────────────────────────────────┴───────────────────┴─────────────────+
                                    │
                      SharedStorageBus (Async Mutex)
                                    │
           ┌────────────────────────┼────────────────────────┐
           ▼                        ▼                        ▼
  EmbassyOverlayEngine          VfsReader              PartitionView
           │                        │                        │
           ▼                        ▼                        ▼
  SRAM Execution Slots      DMA Framebuffer/Audio     Non-Volatile Map
```

---

## Quickstart

### 1. Transparent Embassy Flow (`feature = "embassy"`, Recommended)

```rust
use embassy_executor::Spawner;
use embedded_overlay::embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embedded_overlay::embassy::{
    define_overlay_slots, EmbassyOverlayEngine, SharedStorageBus,
};
use embedded_overlay::memory_overlay;
use embedded_overlay::partition::PartitionView;
use embedded_overlay::vfs::VfsReader;

// 1. Transparently define overlay functions with memory_overlay!
// Zero manual IDs! The compiler automatically hashes "compute_physics" to derive its
// 32-bit ID at compile time and links its machine code into section .overlay.compute_physics.
memory_overlay! {
    pub async fn compute_physics(velocity: i32, dt: i32) -> i32 {
        velocity + (dt * 98) / 10
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let flash = MySpiFlashDriver::new(...);

    // 2. Shared bus allows overlays, VFS, and sequential-storage to run concurrently!
    let bus = SharedStorageBus::<CriticalSectionRawMutex, _>::new(flash);

    // 3. Safe static slot allocation with zero unsafe code and guaranteed 16-byte alignment
    define_overlay_slots!(slots, 2, 16384);

    // 4. Initialize engine and auto-mount all overlays from external flash:
    // No manual IDs, no manual flash offset tracking!
    let mut engine = EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 2>::new(bus.handle(), slots);
    let count = engine.mount(0).await.unwrap(); // Automatically discovers all modules!

    // 5. Concurrently mount VFS and partition views using the exact same bus handle:
    let mut vfs = VfsReader::mount(bus.handle(), 0x0020_0000).await.unwrap();
    let mut part = PartitionView::new(bus.handle(), 0x00E0_0000, 64 * 1024);

    // 6. Direct transparent async call: no slot indices, no flash offsets, no manual packaging!
    let updated_velocity = compute_physics(&engine, 100, 2).await.unwrap();

    // 7. Asynchronous prefetch in background while Embassy tasks continue running:
    engine.prefetch::<compute_physics::Module>().await.unwrap();
}
```

---

### 2. Zero-Touch `cargo run` Workflow

You **never need to manually create `.ovl` files or track flash offsets**.

Configure your project's `.cargo/config.toml`:
```toml
[target.thumbv8m.main-none-eabihf]
runner = "python3 ../../tools/flash_dual.py auto-run --chip STM32WBA65RI"
```

Then simply run:
```bash
cargo run
```

The auto-runner transparently:
1. Inspects the compiled ELF binary and detects all `.overlay.*` sections.
2. Automatically extracts each section's raw machine code with `llvm-objcopy`.
3. Derives 32-bit FNV-1a IDs and computes IEEE CRC32 checksums.
4. Builds an `ext_flash.bin` image prefixed with an [`OverlayDirectory`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/header.rs#L254) table (`b"OVLD"`).
5. Creates a unified `firmware.fwbundle` container.
6. Launches `probe-rs run` to program internal flash and stream live `defmt`/RTT output.
7. On MCU boot, `engine.mount(0).await` reads the directory table and registers all modules!

---

### 3. Automated Linker Script Generation (`build.rs`)

If you want the linker to map memory regions (VMA in SRAM slots, LMA in external flash) at compile time:

```rust
// In your application's build.rs:
use embedded_overlay::build::OverlayLinkerConfig;
use std::env;
use std::path::PathBuf;

fn main() {
    let config = OverlayLinkerConfig {
        flash_origin: 0x0801_0000,
        flash_length: 512 * 1024,
        ram_origin: 0x2000_0000,
        ram_length: 384 * 1024,
        slot_a_origin: 0x2006_0000,
        slot_a_length: 32 * 1024,
        slot_b_origin: 0x2006_8000,
        slot_b_length: 32 * 1024,
        ext_flash_origin: 0x9000_0000,
        ext_flash_length: 16 * 1024 * 1024,
    };

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    config.emit_to_file(out_dir.join("memory-overlay.x")).unwrap();
}
```

---

### 4. Double-Buffered Ping-Pong Streaming & Prefetching

Execute compute kernels in Slot A while asynchronously streaming the next module into Slot B over SPI DMA with zero CPU stall:

```rust
use embedded_overlay::embassy::call_and_prefetch;

// Executes ModuleA in Slot A and automatically triggers background prefetch of ModuleB into Slot B
let result_a = call_and_prefetch::<ModuleA, ModuleB, _, _, 2, 16, _>(&engine, input_a).await.unwrap();

// Switchover to the already-prefetched Slot B:
let result_b = engine.call::<ModuleB>(input_b).await.unwrap();
```

---

### 5. Streamable VFS for Textures, Maps, and Audio

Stream multi-megabyte binary assets directly into destination RAM buffers without intermediate heap allocation:

```rust
use embedded_overlay::vfs::VfsReader;

// Mount the VFS partition at offset 2 MB
let mut vfs = VfsReader::mount(bus.handle(), 0x0020_0000).await.unwrap();

// Stream a 512-byte scanline directly into the display DMA framebuffer
let mut scanline = [0u8; 512];
let bytes_read = vfs.stream_chunk(ASSET_DOOM_TEXTURE, offset, &mut scanline).await.unwrap();
```

---

### 6. MicroSD Card Support (SPI / SDIO Block Devices)

Because microSD cards use 512-byte blocks with internal wear-leveling, [`BlockDeviceAdapter`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/block.rs) provides a zero-cost bridge from any [`AsyncBlockDevice`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/block.rs) to standard [`ReadNorFlash`](https://docs.rs/embedded-storage-async):

```rust
use embedded_overlay::block::BlockDeviceAdapter;

// 1. Initialize your microSD SPI driver (e.g. embedded-sdmmc or raw SPI SD card)
let sd_card = MySpiSdCard::new(spi, cs);

// 2. Wrap it with the 512-byte block adapter
let mut flash_adapter = BlockDeviceAdapter::<_, 512>::new(sd_card);

// 3. Mount overlays and VFS directly off the microSD card!
let bus = SharedStorageBus::<CriticalSectionRawMutex, _>::new(flash_adapter);
let mut engine = EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 2>::new(bus.handle(), slots);
let mut vfs = VfsReader::mount(bus.handle(), 0x0010_0000).await.unwrap();
```

* **Whole-Block Zero-Copy DMA**: Aligned 512-byte reads stream directly into RAM over SPI DMA.
* **Sub-Block Bounce Buffer**: Handles unaligned reads (e.g. 32-byte overlay headers or 16-byte VFS index records).

---

### 7. Partitioning for `sequential-storage` & `cfg-noodle`

Slice physical external flash or SD card space into isolated logical partitions for wear-leveled key-value persistence:

```rust
use embedded_overlay::partition::PartitionView;
use sequential_storage::map::{MapConfig, MapStorage};
use sequential_storage::cache::Cache;

// Create an isolated 64 KB partition on external flash
let part = PartitionView::new(bus.handle(), 0x00E0_0000, 64 * 1024);

// Pass the partition directly to sequential-storage
let config = MapConfig::new(0..64 * 1024);
let mut storage = MapStorage::new(part, config, Cache::new_uncached());
storage.store_item(&mut buf, &KEY_BLE_BOND, &bond_data).await.unwrap();
```

---

### 8. Low-Level / Bare-Metal Mode (Without Embassy)

If running in a bare-metal environment without Embassy or `embassy-sync`, you can use the direct [`OverlayManager`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/overlay/mod.rs):

```rust
use embedded_overlay::overlay::{OverlayManager, OverlaySlot};

#[repr(align(16))]
struct SlotBuffer([u8; 16 * 1024]);
static mut SLOT_A_BUF: SlotBuffer = SlotBuffer([0; 16 * 1024]);
static mut SLOT_B_BUF: SlotBuffer = SlotBuffer([0; 16 * 1024]);

let slots = [
    OverlaySlot::from_static_slice(unsafe { &mut SLOT_A_BUF.0 }),
    OverlaySlot::from_static_slice(unsafe { &mut SLOT_B_BUF.0 }),
];

let mut overlay_mgr = OverlayManager::<_, 2>::new(flash, slots);
let slot_idx = overlay_mgr.ensure_resident_typed::<PhysicsEngine>(0x0000).await.unwrap();
let result = overlay_mgr.call_typed::<PhysicsEngine>(slot_idx, current_state).unwrap();
```

---

### 9. Instruction-Cache Coherency (Cortex-M7 / STM32 ICACHE)

After machine code is written into a RAM slot, the core must observe it before it is executed. `OverlayManager` performs this through the [`InstructionCacheSync`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/overlay/sync.rs) hook, which is invoked exactly once per slot load:

* The default (used by `OverlayManager::new`) issues `DSB` + `ISB`. This is correct for cores with no cache in front of the code bus (Cortex-M0/M0+/M3/M4).
* `DSB`/`ISB` do **not** invalidate a hardware instruction cache. Two common cases:

**Cortex-M7 (core I-cache)** — use the built-in [`CoreIcacheSync`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/overlay/sync.rs), available with the `cortex-m` feature on Armv7-M/Armv8-M targets. It runs `DSB` -> `SCB::ICIALLU` -> `DSB` -> `ISB`:

```rust
use embedded_overlay::{CoreIcacheSync, OverlayManager};

let manager = OverlayManager::<_, 2, _>::with_sync(flash, slots, CoreIcacheSync);
```

**STM32 ICACHE block (WBA/U5/H5/U3)** — the register-level driver lives in the HAL; implement the hook on top of it:

```rust
use embedded_overlay::InstructionCacheSync;

struct IcacheSync {
    icache: embassy_stm32::icache::Icache<'static>,
}

impl InstructionCacheSync for IcacheSync {
    fn code_loaded(&mut self) {
        cortex_m::asm::dsb();
        // Full-cache CACHEINV. DSB/ISB alone cannot invalidate the ICACHE block.
        self.icache.invalidate();
        cortex_m::asm::isb();
    }
}

let mut engine =
    EmbassyOverlayEngine::<CriticalSectionRawMutex, _, 2, 16, IcacheSync>::with_sync(
        bus.handle(),
        slots,
        IcacheSync { icache },
    );
```

> If the Cortex-M7 **D-cache** is enabled and the slot was filled by CPU stores rather than DMA, clean the D-cache for the written range first, or map the slot region as non-cacheable in the MPU.

To fetch overlay code *through* the cache, configure the HAL's ICACHE remap (a code-region alias window) and point the slot's execution address at that alias with [`OverlaySlot::with_exec_addr`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/overlay/slot.rs), while code is still written to its physical SRAM address. Without a remap, SRAM is fetched over the system bus and no cache maintenance is required. A complete reference is in [`demos/wba65-overlay-demo`](file:///home/usuario/Projects/my-repos/embedded-overlay/demos/wba65-overlay-demo/src/main.rs).

---

## Interoperability with Operating Systems

### Ariel OS
[Ariel OS](https://github.com/ariel-os/ariel-os) is an operating system for secure, low-power IoT microcontrollers. While Ariel OS provides multitasking (`#[ariel_os::task]`, `#[ariel_os::thread]`) and on-chip flash key-value storage (`ariel_os::storage`), it does not have abstractions for external NOR flash/SD card filesystems or dynamic code overlays.

`embedded-overlay` complements Ariel OS by providing:
* **External Storage**: Wrap Ariel OS's SPI peripheral in [`SharedStorageBus`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/shared_storage.rs) or [`BlockDeviceAdapter`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/block.rs) to access external flash chips or SD cards.
* **Dynamic Code Overlays**: Execute compute kernels in RAM on-demand from external media inside an `#[ariel_os::task]` or `#[ariel_os::thread]`.
* **Asset Streaming**: Use [`VfsReader`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/vfs/mod.rs) to stream textures, sounds, and models without consuming on-chip flash.

### Embassy
Full native integration is provided via `features = ["embassy"]`. All drivers and managers integrate with `embassy-sync` mutexes and `embassy-futures`.

---

## Cargo Features

| Feature | Description | Default |
|---|---|---|
| `embassy` | Enables [`EmbassyOverlayEngine`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/engine.rs), [`SharedStorageBus`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/shared_storage.rs), [`define_overlay_slots!`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/slots.rs), and transparent dispatch | No |
| `cortex-m` | Enables `cortex-m` barriers (`DSB`/`ISB`) for instruction synchronization. This is *not* an instruction-cache invalidate; on Cortex-M7 or STM32 ICACHE parts, supply an [`InstructionCacheSync`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/overlay/sync.rs) implementation via `OverlayManager::with_sync` / `EmbassyOverlayEngine::with_sync` | No |
| `defmt` | Enables formatting implementations for `defmt` logging | No |
| `portable-atomics` | Enables atomic compare-and-swap on targets without native CAS (Cortex-M0/M0+, `thumbv6m-none-eabi`) via `portable-atomic`'s `critical-section` fallback. Required when using `embassy` on `thumbv6m`, since `static_cell` needs CAS | No |
| `std` | Enables standard library support and host `overlay-packer` CLI | No |

> **Cortex-M0/M0+ (`thumbv6m-none-eabi`):** the target has no atomic CAS, which the `embassy` feature's `static_cell` dependency requires. Enable `portable-atomics` alongside `embassy`:
> ```toml
> embedded-overlay = { version = "0.1", features = ["embassy", "portable-atomics"] }
> ```
> This makes `portable-atomic` use critical sections for CAS, so you must also provide a `critical-section` implementation (e.g. `cortex-m`'s `critical-section-single-core` feature), which an Embassy Cortex-M0 application needs anyway.

---

## Tooling Reference

### 1. Dual-Memory Flashing Tool ([`tools/flash_dual.py`](file:///home/usuario/Projects/my-repos/embedded-overlay/tools/flash_dual.py))

```bash
# 1. Zero-touch auto-run (Used by cargo run):
./tools/flash_dual.py auto-run --chip STM32WBA65RI target/thumbv8m.main-none-eabihf/release/wba65-overlay-demo

# 2. Package unified firmware bundle manually:
./tools/flash_dual.py pack --internal app.bin --external external.bin --out firmware.fwbundle

# 3. Flash on-chip internal flash via SWD:
./tools/flash_dual.py flash-internal target/thumbv8m.main-none-eabihf/release/wba65-overlay-demo

# 4. Flash external SPI flash via bootloader bridge:
./tools/flash_dual.py flash-external --port /dev/ttyACM0 external.bin
```

### 2. Rust Packaging CLI ([`overlay-packer`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/bin/packer.rs))

```bash
# Automatically extract all .overlay.* sections from an ELF into external flash image:
cargo run --features std --bin overlay-packer -- auto-pack target/.../my-app ext_flash.bin

# Package an individual binary into an .ovl container with CRC32:
cargo run --features std --bin overlay-packer -- overlay physics_kernel.bin 0x1001 physics.ovl

# Package an asset directory into a contiguous streamable .vfs image:
cargo run --features std --bin overlay-packer -- vfs ./assets ./assets.vfs

# Create a unified dual-flash firmware bundle (.fwbundle):
cargo run --features std --bin overlay-packer -- bundle app.bin external.bin STM32WBA65RI 0x08010000 0x00000000 firmware.fwbundle
```

---

## Ready-to-Run STM32WBA65 Demo ([`demos/wba65-overlay-demo/`](file:///home/usuario/Projects/my-repos/embedded-overlay/demos/wba65-overlay-demo/))

A complete demonstration is available in [`demos/wba65-overlay-demo/`](file:///home/usuario/Projects/my-repos/embedded-overlay/demos/wba65-overlay-demo):
* Uses [`memory_overlay!`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/macros.rs) to compile physics calculation code into `.overlay.compute_physics`.
* Wraps storage in [`SharedStorageBus`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/shared_storage.rs) so overlays, VFS, and sequential-storage run concurrently.
* Auto-discovers and registers overlays on boot via [`engine.mount(0).await`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/embassy/engine.rs#L49-L135).
* Streams asset chunks using [`VfsReader`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/vfs/mod.rs).
* Persists runtime telemetry using [`sequential-storage`](https://crates.io/crates/sequential-storage) on [`PartitionView`](file:///home/usuario/Projects/my-repos/embedded-overlay/src/partition.rs).
* Concurrently runs Embassy async background tasks (`heartbeat_task`).

```bash
cd demos/wba65-overlay-demo
cargo run
```

---

## Verification & Test Suite

The test suite validates all features on host and target:
```bash
cargo test --all-features
```
* CRC32 IEEE 802.3 and compile-time FNV-1a hashing.
* Transparent overlay execution and argument passing.
* Automated directory mounting (`OVLD`) and sequential container auto-discovery (`OVL1`).
* Concurrent access via `SharedStorageBus` without deadlocks.
* MicroSD 512-byte block adapter aligned and unaligned accesses.
* VFS superblock parsing, index lookups, and chunk streaming.
* PartitionView isolation and `sequential-storage` map persistence.

---

## License

Dual-licensed under either of:
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT license ([LICENSE-MIT](LICENSE-MIT))
