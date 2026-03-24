//! Generic retry-with-backoff executor.
//!
//! Replaces duplicated retry loops in LLM providers and adapter reconnect
//! logic.

use std::{future::Future, time::Duration};

/// Retry an async operation with exponential backoff.
///
/// - `max_retries`: maximum number of retries (0 = single attempt, no retries).
/// - `base_ms`: base delay in milliseconds before doubling.
/// - `max_ms`: cap on delay.
/// - `f`: closure returning a future that produces `Result<T, E>`.
/// - `should_retry`: predicate on the error to decide whether to retry.
///
/// Returns the first `Ok` result or the last `Err` after exhausting retries.
pub async fn retry_with_backoff<T, E, F, Fut, R>(
    max_retries: u32,
    base_ms: u64,
    max_ms: u64,
    mut f: F,
    should_retry: R,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    R: Fn(&E) -> bool,
{
    let mut last_err = None;
    for attempt in 0..=max_retries {
        if attempt > 0 {
            let delay_ms =
                base_ms.saturating_mul(1u64.checked_shl(attempt - 1).unwrap_or(u64::MAX));
            let delay = Duration::from_millis(delay_ms.min(max_ms));
            tokio::time::sleep(delay).await;
        }
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if attempt < max_retries && should_retry(&e) {
                    last_err = Some(e);
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last_err.expect("at least one attempt must be made"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[tokio::test]
    async fn succeeds_immediately() {
        let result: Result<i32, &str> =
            retry_with_backoff(3, 10, 100, || async { Ok(42) }, |_| true).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let attempts = AtomicU32::new(0);
        let result: Result<&str, &str> = retry_with_backoff(
            3,
            1,
            10,
            || {
                let n = attempts.fetch_add(1, Ordering::SeqCst);
                async move { if n < 2 { Err("fail") } else { Ok("ok") } }
            },
            |_| true,
        )
        .await;
        assert_eq!(result.unwrap(), "ok");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn exhausts_retries() {
        let result: Result<(), &str> =
            retry_with_backoff(2, 1, 10, || async { Err("always fail") }, |_| true).await;
        assert_eq!(result.unwrap_err(), "always fail");
    }

    #[tokio::test]
    async fn non_retryable_error_returns_immediately() {
        let attempts = AtomicU32::new(0);
        let result: Result<(), &str> = retry_with_backoff(
            5,
            1,
            10,
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("fatal") }
            },
            |_| false,
        )
        .await;
        assert_eq!(result.unwrap_err(), "fatal");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}
