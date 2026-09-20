//! A hardware-agnostic `no_std` runtime library enabling microcontrollers across the
//! entire ARM Cortex-M family:
//! - **Cortex-M0 / M0+** (`thumbv6m-none-eabi`)
//! - **Cortex-M3** (`thumbv7m-none-eabi`)
//! - **Cortex-M4 / M4F** (`thumbv7em-none-eabi` / `thumbv7em-none-eabihf`)
//! - **Cortex-M7** (`thumbv7em-none-eabihf`)
//! - **Cortex-M23 / M33** (`thumbv8m.main-none-eabihf`)
//!
//! To break through on-chip flash memory limits using:
//!
//! 1. **RAM-Paged Code Overlays with Asynchronous DMA Streaming**:
//!    Load and execute Thumb-2 machine code modules dynamically in dedicated SRAM slots
//!    on-demand, yielding the CPU to concurrent tasks during SPI DMA transfers.
//! 2. **Native Embassy & OS Integration (`feature = "embassy"`)**:
//!    Provides [`EmbassyOverlayEngine`] for transparent overlay execution without managing
//!    SRAM slot indices or flash byte offsets, [`SharedStorageBus`] to eliminate storage
//!    ownership contention, and [`define_overlay_slots!`] for safe static allocation.
//! 3. **Double-Buffered Ping-Pong Execution & Prefetching**:
//!    Execute compute kernels in Slot A while asynchronously pre-fetching the next
//!    module into Slot B over SPI DMA with zero CPU blocking.
//! 4. **Streamable Virtual Filesystem (VFS)**:
//!    Stream large binary assets (textures, game maps, audio clips, neural network weights)
//!    directly from external SPI NOR flash or SD card into RAM buffers with zero copying overhead.
//! 5. **MicroSD Card & Block Device Support**:
//!    Zero-cost block bridging via [`BlockDeviceAdapter`] converts 512-byte block media
//!    (such as SPI SD cards) into standard NOR flash interfaces.
//! 6. **Partition Adapter for Ecosystem Crates**:
//!    Slice physical external storage into isolated logical partitions, exposing standard
//!    [`embedded_storage::nor_flash::NorFlash`] traits to [`sequential-storage`](https://crates.io/crates/sequential-storage) and
//!    `cfg-noodle` for wear-leveled non-volatile configuration.
//!
//! ## Decoupled Hardware Architecture
//!
//! `embedded-overlay` contains **zero hardcoded GPIO pins or MCU peripherals**. It is
//! fully generic over [`embedded_storage_async::nor_flash::ReadNorFlash`]. The application
//! binary configures the concrete SPI driver (e.g. `embassy_stm32::spi::Spi` with DMA
//! and `is25lp128f`) and passes it into [`OverlayManager`] or [`EmbassyOverlayEngine`].
//!
//! ## Interoperability with Operating Systems (Ariel OS & Embassy)
//!
//! Operating systems such as [Ariel OS](https://github.com/ariel-os/ariel-os) provide
//! task scheduling and on-chip key-value persistence, but lack external storage filesystems
//! and dynamic code overlays. `embedded-overlay` integrates natively with Embassy and Ariel OS
//! via [`SharedStorageBus`] and [`EmbassyOverlayEngine`], allowing external NOR flash and SD cards
//! to serve as an expanded executable and filesystem area.

#![no_std]

pub mod block;
pub mod bundle;
pub mod crc;
pub mod error;
pub mod header;
pub mod mock;
pub mod overlay;
pub mod partition;
pub mod vfs;

pub use embedded_storage_async;

#[cfg(feature = "embassy")]
pub use embassy_sync;

#[macro_use]
pub mod macros;

// The linker-script generator is host-side tooling (it runs in an application's `build.rs` and
// uses `std`), so it is not compiled for bare-metal targets. That keeps `--all-features`
// buildable for the Cortex-M targets in `package.metadata.docs.rs`.
#[cfg(all(feature = "build", not(target_os = "none")))]
pub mod build;

#[cfg(feature = "embassy")]
pub mod embassy;

pub use block::{AsyncBlockDevice, BlockAdapterError, BlockDeviceAdapter};
pub use bundle::BundleHeader;
pub use crc::{crc32, fnv1a_hash};
pub use error::{OverlayError, VfsError};
pub use header::{OverlayHeader, VfsAssetEntry, VfsSuperblock};
pub use overlay::{InstructionCacheSync, OverlayEntryFn, OverlayManager, OverlayModule, OverlaySlot};
#[cfg(all(feature = "cortex-m", target_arch = "arm", target_has_atomic = "ptr"))]
pub use overlay::CoreIcacheSync;
pub use partition::{PartitionError, PartitionView};
pub use vfs::{LruSectorCache, VfsReader};

#[cfg(feature = "embassy")]
pub use embassy::{
    call_and_prefetch, EmbassyOverlayEngine, ModuleRegistration, SharedStorageBus,
    SharedStorageHandle,
};
