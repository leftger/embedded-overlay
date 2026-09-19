//! Typed traits and invocation signatures for overlay modules.

/// Trait defining a typed overlay module contract.
///
/// Implement this trait on zero-sized marker types to enforce compile-time
/// type safety when calling dynamically loaded RAM overlays.
///
/// # Safety
/// The `MODULE_ID` must match the `module_id` in the corresponding `.ovl`
/// binary in flash, and the compiled entry point signature must match
/// `unsafe extern "C" fn(Self::Args) -> Self::Output`.
pub unsafe trait OverlayModule {
    /// Unique 32-bit module identifier.
    const MODULE_ID: u32;

    /// Arguments passed to the module's entry point.
    type Args;

    /// Return value returned from the module's entry point.
    type Output;
}

/// Standard C-ABI entry function pointer signature for overlays.
pub type OverlayEntryFn<Args, Output> = unsafe extern "C" fn(Args) -> Output;
