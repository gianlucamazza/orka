//! Type aliases used across the core domain model.

use super::{envelope::Envelope, ids::SessionId};

/// Type alias for the message sink passed to channel adapters.
///
/// Deprecated by [`InteractionSink`]; kept for internal use by the bus/worker.
pub type MessageSink = tokio::sync::mpsc::Sender<Envelope>;

/// Type alias for the interaction sink passed to channel adapters.
///
/// Adapters produce [`orka_contracts::InboundInteraction`] and send it to this
/// sink. The bridge in `orka-server` converts to [`Envelope`] for the bus.
pub type InteractionSink = tokio::sync::mpsc::Sender<orka_contracts::InboundInteraction>;

/// Type alias for the message stream returned by the bus.
pub type MessageStream = tokio::sync::mpsc::Receiver<Envelope>;

/// Shared map from session ID to active generation cancellation token.
///
/// The worker registers a token before each dispatch; the `/cancel` endpoint
/// uses it to abort an in-progress generation without stopping the worker.
pub type SessionCancelTokens = std::sync::Arc<
    std::sync::Mutex<std::collections::HashMap<SessionId, tokio_util::sync::CancellationToken>>,
>;

/// Exponential backoff delay with full jitter, capped at `max_secs`.
///
/// Computes a ceiling of `base_secs * 2^attempt` (capped at `max_secs`), then
/// returns a duration in `[0, ceiling]` using subsecond system-clock entropy.
/// This prevents thundering-herd retry storms when multiple workers fail
/// simultaneously, without requiring an external PRNG dependency.
pub fn backoff_delay(attempt: u32, base_secs: u64, max_secs: u64) -> std::time::Duration {
    let secs = base_secs.saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
    let ceiling = secs.min(max_secs);
    let jittered = if ceiling > 0 {
        let nanos = u64::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos(),
        );
        nanos % (ceiling + 1)
    } else {
        0
    };
    std::time::Duration::from_secs(jittered)
}
