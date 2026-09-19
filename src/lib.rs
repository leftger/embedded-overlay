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
//!    on-demand, yielding the CPU to concurrent Embassy tasks during SPI DMA transfers.
//! 2. **Double-Buffered Ping-Pong Execution & Prefetching**:
//!    Execute compute kernels in Slot A while asynchronously pre-fetching the next
//!    module into Slot B over SPI DMA with zero CPU blocking.
//! 3. **Streamable Virtual Filesystem (VFS)**:
//!    Stream large binary assets (textures, game maps, audio clips, neural network weights)
//!    directly from external SPI NOR flash into RAM buffers with zero copying overhead.
//! 4. **Partition Adapter for Ecosystem Crates**:
//!    Slice physical external flash into isolated logical partitions, exposing standard
//!    [`embedded_storage::nor_flash::NorFlash`] traits to [`sequential-storage`] and
//!    `cfg-noodle` for wear-leveled non-volatile configuration.
//!
//! ## Decoupled Hardware Architecture
//!
//! `embedded-overlay` contains **zero hardcoded GPIO pins or MCU peripherals**. It is
//! fully generic over [`embedded_storage_async::nor_flash::ReadNorFlash`]. The application
//! binary configures the concrete SPI driver (e.g. `embassy_stm32::spi::Spi` with DMA
//! and `is25lp128f`) and passes it into [`OverlayManager`].

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

pub use block::{AsyncBlockDevice, BlockAdapterError, BlockDeviceAdapter};
pub use bundle::BundleHeader;
pub use crc::crc32;
pub use error::{OverlayError, VfsError};
pub use header::{OverlayHeader, VfsAssetEntry, VfsSuperblock};
pub use overlay::{OverlayEntryFn, OverlayManager, OverlayModule, OverlaySlot};
pub use partition::{PartitionError, PartitionView};
pub use vfs::{LruSectorCache, VfsReader};
