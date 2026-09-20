//! Double-buffered ping-pong pipelining helpers for Embassy concurrency.

use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_storage_async::nor_flash::ReadNorFlash;

use crate::error::OverlayError;
use crate::overlay::{InstructionCacheSync, OverlayModule};
use super::engine::EmbassyOverlayEngine;

/// Executes module `Current` and immediately triggers background prefetch of `Next`.
///
/// Ensures continuous ping-pong throughput with zero CPU stall between execution stages.
pub async fn call_and_prefetch<
    Current: OverlayModule,
    Next: OverlayModule,
    M: RawMutex,
    S: ReadNorFlash,
    const SLOTS: usize,
    const MAX_MODULES: usize,
    B: InstructionCacheSync,
>(
    engine: &EmbassyOverlayEngine<M, S, SLOTS, MAX_MODULES, B>,
    args: Current::Args,
) -> Result<Current::Output, OverlayError> {
    let result = engine.call::<Current>(args).await?;
    let _ = engine.prefetch::<Next>().await?;
    Ok(result)
}
