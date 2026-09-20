//! Declarative macros for transparent overlay function definition and dispatch.

/// Transparently defines an overlay function that can be executed on demand from external storage.
///
/// The module ID is **automatically derived at compile time** from the function name using a
/// 32-bit FNV-1a hash, requiring **zero manual ID management**. If desired for legacy or binary
/// protocol compatibility, an explicit ID can optionally be provided via `id = <value>`.
///
/// This macro eliminates all manual boilerplate:
/// 1. Declares a type-safe [`OverlayModule`](crate::overlay::OverlayModule) contract.
/// 2. Packs function arguments into a C-ABI struct.
/// 3. Creates the compiled machine code entry point.
/// 4. Generates a transparent, strongly-typed async function callable directly by user code.
///
/// # Example (Fully Automatic ID)
/// ```ignore
/// use embedded_overlay::memory_overlay;
///
/// // No manual ID needed! The compiler hashes the name at compile time.
/// memory_overlay! {
///     pub async fn compute_physics(velocity: i32, dt: i32) -> i32 {
///         velocity + (dt * 98) / 10
///     }
/// }
///
/// // In your application:
/// let result = compute_physics(&engine, 100, 2).await.unwrap();
/// ```
#[macro_export]
macro_rules! memory_overlay {
    // Pattern 1: Automatic ID (with return type)
    (
        $(#[$meta:meta])*
        $vis:vis async fn $fn_name:ident ( $($arg_name:ident : $arg_type:ty),* $(,)? ) -> $ret_type:ty $body:block
    ) => {
        $crate::memory_overlay! {
            id = $crate::crc::fnv1a_hash(stringify!($fn_name)),
            $(#[$meta])*
            $vis async fn $fn_name ( $($arg_name : $arg_type),* ) -> $ret_type $body
        }
    };

    // Pattern 2: Automatic ID (no return type, returns ())
    (
        $(#[$meta:meta])*
        $vis:vis async fn $fn_name:ident ( $($arg_name:ident : $arg_type:ty),* $(,)? ) $body:block
    ) => {
        $crate::memory_overlay! {
            id = $crate::crc::fnv1a_hash(stringify!($fn_name)),
            $(#[$meta])*
            $vis async fn $fn_name ( $($arg_name : $arg_type),* ) -> () $body
        }
    };

    // Pattern 3: Explicit ID (with return type)
    (
        id = $id:expr,
        $(#[$meta:meta])*
        $vis:vis async fn $fn_name:ident ( $($arg_name:ident : $arg_type:ty),* $(,)? ) -> $ret_type:ty $body:block
    ) => {
        #[allow(non_snake_case)]
        $vis mod $fn_name {
            use super::*;

            /// Module marker type implementing [`OverlayModule`](crate::overlay::OverlayModule).
            pub struct Module;

            impl Module {
                /// Automatically derived 32-bit module ID.
                pub const MODULE_ID: u32 = $id;
            }

            /// Inherent module ID constant.
            pub const MODULE_ID: u32 = $id;

            /// Arguments struct for C-ABI entry point.
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct Args {
                $(pub $arg_name: $arg_type),*
            }

            unsafe impl $crate::overlay::OverlayModule for Module {
                const MODULE_ID: u32 = $id;
                type Args = Args;
                type Output = $ret_type;
            }

            /// Concrete C-ABI entry point for this overlay.
            #[inline(never)]
            #[link_section = concat!(".overlay.", stringify!($fn_name))]
            pub unsafe extern "C" fn entry_point(args: Args) -> $ret_type {
                let Args { $($arg_name),* } = args;
                $body
            }

            #[used]
            static _KEEP_ENTRY: unsafe extern "C" fn(Args) -> $ret_type = entry_point;

            /// Call this overlay via an [`EmbassyOverlayEngine`](crate::embassy::EmbassyOverlayEngine).
            pub async fn call<
                M,
                S,
                const SLOTS: usize,
                const MAX_MODULES: usize,
                B: $crate::overlay::InstructionCacheSync,
            >(
                engine: &$crate::embassy::EmbassyOverlayEngine<M, S, SLOTS, MAX_MODULES, B>,
                $($arg_name: $arg_type),*
            ) -> Result<$ret_type, $crate::error::OverlayError>
            where
                M: $crate::embassy_sync::blocking_mutex::raw::RawMutex,
                S: $crate::embedded_storage_async::nor_flash::ReadNorFlash,
            {
                let args = Args { $($arg_name),* };
                engine.call::<Module>(args).await
            }
        }

        // Direct async callable function in the current scope
        $(#[$meta])*
        #[inline(always)]
        $vis async fn $fn_name<
            M,
            S,
            const SLOTS: usize,
            const MAX_MODULES: usize,
            B: $crate::overlay::InstructionCacheSync,
        >(
            engine: &$crate::embassy::EmbassyOverlayEngine<M, S, SLOTS, MAX_MODULES, B>,
            $($arg_name: $arg_type),*
        ) -> Result<$ret_type, $crate::error::OverlayError>
        where
            M: $crate::embassy_sync::blocking_mutex::raw::RawMutex,
            S: $crate::embedded_storage_async::nor_flash::ReadNorFlash,
        {
            $fn_name::call(engine, $($arg_name),*).await
        }
    };

    // Pattern 4: Explicit ID (no return type, returns ())
    (
        id = $id:expr,
        $(#[$meta:meta])*
        $vis:vis async fn $fn_name:ident ( $($arg_name:ident : $arg_type:ty),* $(,)? ) $body:block
    ) => {
        $crate::memory_overlay! {
            id = $id,
            $(#[$meta])*
            $vis async fn $fn_name ( $($arg_name : $arg_type),* ) -> () $body
        }
    };
}
