use embedded_overlay::crc::crc32;
use embedded_overlay::header::OverlayHeader;
use embedded_overlay::mock::MockFlash;
use embedded_overlay::overlay::{InstructionCacheSync, OverlayManager, OverlayModule, OverlaySlot};
use embedded_overlay::OverlayError;

// Minimal zero-dependency async executor for host unit tests
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

#[repr(align(16))]
struct AlignedBuffer<const N: usize>([u8; N]);

#[repr(C)]
#[derive(Clone, Copy)]
struct BinaryArgs {
    a: i32,
    b: i32,
}

// Define test functions with standard C calling convention
unsafe extern "C" fn add_numbers(args: BinaryArgs) -> i32 {
    args.a + args.b
}

unsafe extern "C" fn multiply_numbers(args: BinaryArgs) -> i32 {
    args.a * args.b
}

// Marker types for type-safe overlay execution
struct AddModule;
unsafe impl OverlayModule for AddModule {
    const MODULE_ID: u32 = 0x1001;
    type Args = BinaryArgs;
    type Output = i32;
}

struct MulModule;
unsafe impl OverlayModule for MulModule {
    const MODULE_ID: u32 = 0x1002;
    type Args = BinaryArgs;
    type Output = i32;
}

struct HeavyModule;
unsafe impl OverlayModule for HeavyModule {
    const MODULE_ID: u32 = 0x1003;
    type Args = BinaryArgs;
    type Output = i32;
}

/// Helper to write an overlay module into mock flash
fn write_overlay_to_flash(
    flash: &mut MockFlash<65536>,
    offset: u32,
    module_id: u32,
    code_fn: unsafe extern "C" fn(BinaryArgs) -> i32,
) {
    let fn_ptr = code_fn as usize;
    let code_bytes = fn_ptr.to_le_bytes();
    let payload_crc = crc32(&code_bytes);

    let header = OverlayHeader::new(
        module_id,
        code_bytes.len() as u32,
        0, // entry point is at offset 0
        0,
        payload_crc,
    );

    let header_bytes = header.to_bytes();
    let offset = offset as usize;
    flash.data_mut()[offset..offset + OverlayHeader::SIZE].copy_from_slice(&header_bytes);
    flash.data_mut()[offset + OverlayHeader::SIZE..offset + OverlayHeader::SIZE + code_bytes.len()]
        .copy_from_slice(&code_bytes);
}

#[test]
fn test_overlay_loading_and_lru() {
    block_on(async {
        let mut flash = MockFlash::<65536>::new();

        // Module 1 at flash offset 0
        write_overlay_to_flash(&mut flash, 0, AddModule::MODULE_ID, add_numbers);
        // Module 2 at flash offset 256
        write_overlay_to_flash(&mut flash, 256, MulModule::MODULE_ID, multiply_numbers);
        // Module 3 at flash offset 512
        write_overlay_to_flash(&mut flash, 512, HeavyModule::MODULE_ID, add_numbers);

        static mut RAM_SLOT_A: AlignedBuffer<64> = AlignedBuffer([0; 64]);
        static mut RAM_SLOT_B: AlignedBuffer<64> = AlignedBuffer([0; 64]);

        let slots = [
            OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(RAM_SLOT_A.0) }),
            OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(RAM_SLOT_B.0) }),
        ];

        let mut manager = OverlayManager::<_, 2>::new(flash, slots);

        // Initial state: nothing resident
        assert!(!manager.is_resident(AddModule::MODULE_ID));
        assert!(!manager.is_resident(MulModule::MODULE_ID));

        // Load Module 1 -> should go to Slot 0
        let slot1 = manager
            .ensure_resident_typed::<AddModule>(0)
            .await
            .expect("Load module 1");
        assert_eq!(slot1, 0);
        assert!(manager.is_resident(AddModule::MODULE_ID));

        // Load Module 2 -> should go to Slot 1 (Slot 0 is occupied)
        let slot2 = manager
            .ensure_resident_typed::<MulModule>(256)
            .await
            .expect("Load module 2");
        assert_eq!(slot2, 1);
        assert!(manager.is_resident(MulModule::MODULE_ID));

        // Querying resident modules does not reload
        let slot1_again = manager
            .ensure_resident_typed::<AddModule>(0)
            .await
            .expect("Module 1 already resident");
        assert_eq!(slot1_again, 0);

        // Slot 0 (Module 1) was accessed more recently than Slot 1 (Module 2).
        // Now loading Module 3 should evict the LRU slot -> Slot 1 (Module 2)!
        let slot3 = manager
            .ensure_resident_typed::<HeavyModule>(512)
            .await
            .expect("Load module 3 with LRU eviction");
        assert_eq!(slot3, 1);
        assert!(manager.is_resident(HeavyModule::MODULE_ID));
        assert!(!manager.is_resident(MulModule::MODULE_ID));
        assert!(manager.is_resident(AddModule::MODULE_ID));
    });
}

#[test]
fn test_overlay_crc_corruption_detection() {
    block_on(async {
        let mut flash = MockFlash::<65536>::new();
        write_overlay_to_flash(&mut flash, 0, AddModule::MODULE_ID, add_numbers);

        // Corrupt 1 byte in the payload area
        flash.data_mut()[OverlayHeader::SIZE] ^= 0xAA;

        static mut RAM_SLOT: AlignedBuffer<64> = AlignedBuffer([0; 64]);
        let slots = [OverlaySlot::from_static_slice(unsafe {
            &mut *core::ptr::addr_of_mut!(RAM_SLOT.0)
        })];
        let mut manager = OverlayManager::<_, 1>::new(flash, slots);

        let result = manager.ensure_resident_typed::<AddModule>(0).await;
        match result {
            Err(OverlayError::PayloadCrcMismatch { .. }) => {}
            other => panic!("Expected PayloadCrcMismatch, got {:?}", other),
        }

        // Corrupted module must NOT be marked resident
        assert!(!manager.is_resident(AddModule::MODULE_ID));
    });
}

#[test]
fn test_overlay_slot_pinning() {
    block_on(async {
        let mut flash = MockFlash::<65536>::new();
        write_overlay_to_flash(&mut flash, 0, AddModule::MODULE_ID, add_numbers);
        write_overlay_to_flash(&mut flash, 256, MulModule::MODULE_ID, multiply_numbers);
        write_overlay_to_flash(&mut flash, 512, HeavyModule::MODULE_ID, add_numbers);

        static mut RAM_SLOT_A: AlignedBuffer<64> = AlignedBuffer([0; 64]);
        static mut RAM_SLOT_B: AlignedBuffer<64> = AlignedBuffer([0; 64]);

        let slots = [
            OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(RAM_SLOT_A.0) }),
            OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(RAM_SLOT_B.0) }),
        ];

        let mut manager = OverlayManager::<_, 2>::new(flash, slots);

        // Load Module 1 in Slot 0 and PIN it
        manager.ensure_resident_typed::<AddModule>(0).await.unwrap();
        manager.pin_slot(0).unwrap();

        // Load Module 2 in Slot 1
        manager.ensure_resident_typed::<MulModule>(256).await.unwrap();

        // Now load Module 3. Even though Slot 0 is older, it is PINNED, so Slot 1 must be evicted!
        let slot3 = manager
            .ensure_resident_typed::<HeavyModule>(512)
            .await
            .unwrap();
        assert_eq!(slot3, 1);
        assert!(manager.is_resident(AddModule::MODULE_ID)); // Slot 0 protected!
        assert!(manager.is_resident(HeavyModule::MODULE_ID));
        assert!(!manager.is_resident(MulModule::MODULE_ID));
    });
}

/// Instruction-synchronization hook that counts how often the engine invokes it.
struct CountingSync {
    calls: u32,
}

impl InstructionCacheSync for CountingSync {
    fn code_loaded(&mut self) {
        self.calls += 1;
    }
}

#[test]
fn test_instruction_sync_hook_invoked_once_per_load() {
    block_on(async {
        let mut flash = MockFlash::<65536>::new();
        write_overlay_to_flash(&mut flash, 0, AddModule::MODULE_ID, add_numbers);
        write_overlay_to_flash(&mut flash, 256, MulModule::MODULE_ID, multiply_numbers);

        static mut RAM_SLOT_A: AlignedBuffer<64> = AlignedBuffer([0; 64]);
        static mut RAM_SLOT_B: AlignedBuffer<64> = AlignedBuffer([0; 64]);

        let slots = [
            OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(RAM_SLOT_A.0) }),
            OverlaySlot::from_static_slice(unsafe { &mut *core::ptr::addr_of_mut!(RAM_SLOT_B.0) }),
        ];

        let mut manager =
            OverlayManager::<_, 2, _>::with_sync(flash, slots, CountingSync { calls: 0 });
        assert_eq!(manager.sync().calls, 0);

        // First load must synchronize exactly once.
        manager.ensure_resident_typed::<AddModule>(0).await.unwrap();
        assert_eq!(manager.sync().calls, 1);

        // A resident module is not reloaded, so no further synchronization must happen.
        manager.ensure_resident_typed::<AddModule>(0).await.unwrap();
        assert_eq!(manager.sync().calls, 1);

        // Loading a different module synchronizes again.
        manager.ensure_resident_typed::<MulModule>(256).await.unwrap();
        assert_eq!(manager.sync().calls, 2);
    });
}

#[test]
fn test_exec_addr_is_validated_not_ram_addr() {
    block_on(async {
        let mut flash = MockFlash::<65536>::new();
        write_overlay_to_flash(&mut flash, 0, AddModule::MODULE_ID, add_numbers);

        static mut RAM_SLOT: AlignedBuffer<64> = AlignedBuffer([0; 64]);

        // Execution through a misaligned alias must be rejected even though the physical SRAM
        // address itself is 16-byte aligned.
        let slot = OverlaySlot::from_static_slice(unsafe {
            &mut *core::ptr::addr_of_mut!(RAM_SLOT.0)
        })
        .with_exec_addr(0x1000_0002);

        let mut manager = OverlayManager::<_, 1>::new(flash, [slot]);
        let result = manager.ensure_resident_typed::<AddModule>(0).await;
        assert!(
            matches!(result, Err(OverlayError::InvalidAlignment)),
            "expected InvalidAlignment, got {:?}",
            result
        );
    });
}
