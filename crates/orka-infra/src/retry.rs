//! Retry utilities for transient Redis pool errors.

use std::time::Duration;

use deadpool_redis::{Connection, Pool, PoolError};
use tracing::warn;

/// Maximum number of connection attempts (1 initial + 2 retries).
const MAX_ATTEMPTS: u32 = 3;
/// Base delay between retry attempts in milliseconds (doubles each attempt).
const BASE_DELAY_MS: u64 = 50;

/// Acquire a Redis connection from the pool with exponential-backoff retry.
///
/// On transient pool errors the helper waits `50ms × 2^attempt` before the
/// next try, for up to [`MAX_ATTEMPTS`] total attempts.  This smooths over
/// brief Redis restarts or connection-pool exhaustion spikes without failing
/// the caller immediately.
pub async fn get_conn_with_retry(pool: &Pool) -> Result<Connection, PoolError> {
    let mut last_err = None;
    for attempt in 0..MAX_ATTEMPTS {
        match pool.get().await {
            Ok(conn) => return Ok(conn),
            Err(e) => {
                warn!(
                    attempt,
                    error = %e,
                    "redis pool connection failed, retrying"
                );
                last_err = Some(e);
                if attempt + 1 < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(BASE_DELAY_MS * (1 << attempt))).await;
                }
            }
        }
    }
    // SAFETY: last_err is always Some after the loop because MAX_ATTEMPTS > 0
    #[allow(clippy::expect_used)]
    Err(last_err.expect("MAX_ATTEMPTS must be > 0"))
}
