//! Native Embassy integration: async mutex bus sharing, task-sharable Overlay Engine,
//! and safe static slot allocation.

pub mod engine;
pub mod pipeline;
pub mod shared_storage;
pub mod slots;

pub use engine::{EmbassyOverlayEngine, ModuleRegistration};
pub use pipeline::call_and_prefetch;
pub use shared_storage::{SharedStorageBus, SharedStorageHandle};
pub use slots::{init_slot_from_cell, AlignedSlotBuffer};
