//! Safe static overlay slot allocation using `static_cell::StaticCell`.

use crate::overlay::OverlaySlot;
use static_cell::StaticCell;

/// A 16-byte aligned static memory buffer suitable for Thumb-2 machine code execution.
#[repr(align(16))]
pub struct AlignedSlotBuffer<const N: usize>(pub [u8; N]);

impl<const N: usize> AlignedSlotBuffer<N> {
    /// Creates a new zero-initialized aligned slot buffer.
    pub const fn new() -> Self {
        Self([0u8; N])
    }
}

impl<const N: usize> Default for AlignedSlotBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Safely initializes an overlay slot from a static cell without unsafe code,
/// guaranteeing 16-byte alignment.
pub fn init_slot_from_cell<const SIZE: usize>(
    cell: &'static StaticCell<AlignedSlotBuffer<SIZE>>,
) -> OverlaySlot {
    let buf = cell.init(AlignedSlotBuffer::new());
    OverlaySlot::from_static_slice(&mut buf.0)
}

/// Macro to safely define static overlay slot buffers using `static_cell::StaticCell`.
///
/// This eliminates the need for `static mut` arrays and `unsafe` pointer casts,
/// while guaranteeing 16-byte alignment for instruction caches.
///
/// # Example
/// ```ignore
/// use embedded_overlay::define_overlay_slots;
///
/// // Safely declare two 16 KB ping-pong execution slots
/// define_overlay_slots!(slots, 2, 16384);
/// ```
#[macro_export]
macro_rules! define_overlay_slots {
    ($slots_name:ident, 1, $size:expr) => {
        static CELL_0: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        let $slots_name = [$crate::embassy::init_slot_from_cell(&CELL_0)];
    };
    ($slots_name:ident, 2, $size:expr) => {
        static CELL_0: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        static CELL_1: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        let $slots_name = [
            $crate::embassy::init_slot_from_cell(&CELL_0),
            $crate::embassy::init_slot_from_cell(&CELL_1),
        ];
    };
    ($slots_name:ident, 3, $size:expr) => {
        static CELL_0: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        static CELL_1: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        static CELL_2: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        let $slots_name = [
            $crate::embassy::init_slot_from_cell(&CELL_0),
            $crate::embassy::init_slot_from_cell(&CELL_1),
            $crate::embassy::init_slot_from_cell(&CELL_2),
        ];
    };
    ($slots_name:ident, 4, $size:expr) => {
        static CELL_0: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        static CELL_1: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        static CELL_2: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        static CELL_3: ::static_cell::StaticCell<$crate::embassy::AlignedSlotBuffer<$size>> =
            ::static_cell::StaticCell::new();
        let $slots_name = [
            $crate::embassy::init_slot_from_cell(&CELL_0),
            $crate::embassy::init_slot_from_cell(&CELL_1),
            $crate::embassy::init_slot_from_cell(&CELL_2),
            $crate::embassy::init_slot_from_cell(&CELL_3),
        ];
    };
}
