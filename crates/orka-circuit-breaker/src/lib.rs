//! Circuit breaker pattern for protecting remote service calls.
//!
//! [`CircuitBreaker`] tracks consecutive failures and trips open after a
//! threshold, rejecting calls immediately until a cooldown period allows a
//! half-open probe.

#![warn(missing_docs)]

use std::{
    sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
    time::Duration,
};

use tokio::time::Instant;

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum CircuitState {
    /// Normal operation — requests pass through.
    Closed = 0,
    /// Tripped — all requests are rejected immediately.
    Open = 1,
    /// Probing — a single request is allowed to test recovery.
    HalfOpen = 2,
}

impl CircuitState {
    fn from_u8(v: u8) -> Self {
        #[allow(clippy::match_same_arms)]
        match v {
            0 => Self::Closed,
            1 => Self::Open,
            2 => Self::HalfOpen,
            _ => Self::Closed,
        }
    }
}

/// Error returned by [`CircuitBreaker::call`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CircuitBreakerError<E> {
    /// The circuit is open; the call was not attempted.
    #[error("circuit breaker is open")]
    Open,
    /// The inner function returned an error.
    #[error(transparent)]
    Inner(E),
}

/// Configuration for a [`CircuitBreaker`].
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive environmental failures before opening the circuit.
    pub failure_threshold: u32,
    /// Number of consecutive semantic (quality) failures before opening the
    /// circuit.
    ///
    /// Semantic failures come from `validate_output()` returning an error.
    /// Tracked independently of environmental failures so that both thresholds
    /// can be tuned separately. Set to `0` to disable quality-based tripping.
    pub quality_failure_threshold: u32,
    /// Number of consecutive successes in half-open state before closing.
    pub success_threshold: u32,
    /// How long the circuit stays open before transitioning to half-open.
    pub open_duration: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            quality_failure_threshold: 5,
            success_threshold: 2,
            open_duration: Duration::from_secs(30),
        }
    }
}

/// A generic circuit breaker that wraps fallible async operations.
///
/// The breaker is `Send + Sync + 'static` and can be shared across tasks
/// via `Arc`. It tracks two independent failure counters:
/// - `failure_count`: environmental / transient failures
/// - `quality_failures`: semantic (output validation) failures
///
/// The circuit opens when *either* counter reaches its configured threshold.
///
/// Fully lock-free: all state is stored in atomics. `open_since_nanos` encodes
/// the open timestamp as nanoseconds elapsed since `base_instant` (0 = not
/// open). `u64` nanoseconds covers ~584 years, far beyond any practical
/// `open_duration`.
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    /// 0 = Closed, 1 = Open, 2 = `HalfOpen`
    state: AtomicU8,
    failure_count: AtomicU32,
    quality_failures: AtomicU32,
    success_count: AtomicU32,
    half_open_probes: AtomicU32,
    /// Fixed reference point created in `new()`.
    base_instant: Instant,
    /// Nanoseconds since `base_instant` when the circuit opened; 0 = not open.
    open_since_nanos: AtomicU64,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: AtomicU8::new(CircuitState::Closed as u8),
            failure_count: AtomicU32::new(0),
            quality_failures: AtomicU32::new(0),
            success_count: AtomicU32::new(0),
            half_open_probes: AtomicU32::new(0),
            base_instant: Instant::now(),
            open_since_nanos: AtomicU64::new(0),
        }
    }

    /// Execute `f` through the circuit breaker.
    ///
    /// - **Closed**: calls `f`. On failure, increments the failure counter. If
    ///   the threshold is reached the circuit opens.
    /// - **Open**: if `open_duration` has elapsed, transitions to half-open and
    ///   allows the call. Otherwise returns [`CircuitBreakerError::Open`].
    /// - **`HalfOpen`**: allows at most one concurrent probe. On success the
    ///   circuit closes; on failure it opens again.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        let current = CircuitState::from_u8(self.state.load(Ordering::SeqCst));

        match current {
            CircuitState::Closed => self.call_closed(f).await,
            CircuitState::Open => self.call_open(f).await,
            CircuitState::HalfOpen => self.call_half_open(f).await,
        }
    }

    /// Return the current state for observability.
    pub fn state(&self) -> CircuitState {
        let raw = self.state.load(Ordering::SeqCst);

        // Auto-transition from Open to HalfOpen if duration elapsed.
        if raw == CircuitState::Open as u8 && self.open_duration_elapsed() {
            self.state
                .store(CircuitState::HalfOpen as u8, Ordering::SeqCst);
            self.half_open_probes.store(0, Ordering::SeqCst);
            return CircuitState::HalfOpen;
        }

        CircuitState::from_u8(raw)
    }

    /// Record a success without executing a closure (post-hoc feedback).
    ///
    /// Equivalent to what `call()` does internally on success: resets the
    /// failure counter in Closed state, or advances the success counter in
    /// `HalfOpen` state.
    pub fn record_success(&self) {
        let state = CircuitState::from_u8(self.state.load(Ordering::SeqCst));
        match state {
            CircuitState::Closed => {
                self.failure_count.store(0, Ordering::SeqCst);
            }
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if successes >= self.config.success_threshold {
                    tracing::info!("circuit breaker closing after recorded successes");
                    self.reset();
                }
            }
            CircuitState::Open => {}
        }
    }

    /// Record a failure without executing a closure (post-hoc feedback).
    ///
    /// Equivalent to what `call()` does internally on failure: increments the
    /// failure counter and trips the circuit if the threshold is reached.
    pub fn record_failure(&self) {
        let state = CircuitState::from_u8(self.state.load(Ordering::SeqCst));
        match state {
            CircuitState::Closed => {
                let failures = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                if failures >= self.config.failure_threshold {
                    tracing::warn!(
                        failures,
                        threshold = self.config.failure_threshold,
                        "circuit breaker tripped to Open via record_failure"
                    );
                    self.transition_to_open();
                }
            }
            CircuitState::HalfOpen => {
                tracing::warn!("half-open probe failed via record_failure, circuit re-opening");
                self.transition_to_open();
            }
            CircuitState::Open => {}
        }
    }

    /// Record a semantic (quality) failure — output validation rejected the
    /// result.
    ///
    /// Tracked in a separate counter from environmental failures. Trips the
    /// circuit when `quality_failures >= config.quality_failure_threshold`
    /// (unless threshold is 0).
    pub fn record_quality_failure(&self) {
        if self.config.quality_failure_threshold == 0 {
            return;
        }
        let state = CircuitState::from_u8(self.state.load(Ordering::SeqCst));
        if state == CircuitState::Open {
            return;
        }
        let failures = self.quality_failures.fetch_add(1, Ordering::SeqCst) + 1;
        if failures >= self.config.quality_failure_threshold {
            tracing::warn!(
                failures,
                threshold = self.config.quality_failure_threshold,
                "circuit breaker tripped to Open via quality failures"
            );
            self.transition_to_open();
        }
    }

    /// Manually reset the circuit breaker to the Closed state.
    pub fn reset(&self) {
        self.state
            .store(CircuitState::Closed as u8, Ordering::SeqCst);
        self.failure_count.store(0, Ordering::SeqCst);
        self.quality_failures.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        self.half_open_probes.store(0, Ordering::SeqCst);
        self.open_since_nanos.store(0, Ordering::SeqCst);
    }

    // -- private helpers --

    async fn call_closed<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        match f().await {
            Ok(val) => {
                self.failure_count.store(0, Ordering::SeqCst);
                Ok(val)
            }
            Err(e) => {
                let failures = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                if failures >= self.config.failure_threshold {
                    tracing::warn!(
                        failures,
                        threshold = self.config.failure_threshold,
                        "circuit breaker tripped to Open"
                    );
                    self.transition_to_open();
                }
                Err(CircuitBreakerError::Inner(e))
            }
        }
    }

    async fn call_open<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        if self.open_duration_elapsed() {
            // Transition to HalfOpen and allow this call as a probe.
            self.state
                .store(CircuitState::HalfOpen as u8, Ordering::SeqCst);
            self.half_open_probes.store(0, Ordering::SeqCst);
            self.success_count.store(0, Ordering::SeqCst);
            tracing::info!("circuit breaker transitioning to HalfOpen");
            self.call_half_open(f).await
        } else {
            Err(CircuitBreakerError::Open)
        }
    }

    async fn call_half_open<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        // Allow at most 1 concurrent probe.
        let prev = self.half_open_probes.fetch_add(1, Ordering::SeqCst);
        if prev >= 1 {
            self.half_open_probes.fetch_sub(1, Ordering::SeqCst);
            return Err(CircuitBreakerError::Open);
        }

        match f().await {
            Ok(val) => {
                self.half_open_probes.fetch_sub(1, Ordering::SeqCst);
                let successes = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if successes >= self.config.success_threshold {
                    tracing::info!("circuit breaker closing after successful probes");
                    self.reset();
                }
                Ok(val)
            }
            Err(e) => {
                self.half_open_probes.fetch_sub(1, Ordering::SeqCst);
                tracing::warn!("half-open probe failed, circuit breaker re-opening");
                self.transition_to_open();
                Err(CircuitBreakerError::Inner(e))
            }
        }
    }

    fn transition_to_open(&self) {
        self.state.store(CircuitState::Open as u8, Ordering::SeqCst);
        self.failure_count.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        self.half_open_probes.store(0, Ordering::SeqCst);
        // Store elapsed nanos since base_instant (minimum 1 to distinguish from "not
        // open").
        let nanos = self.base_instant.elapsed().as_nanos() as u64;
        self.open_since_nanos.store(nanos.max(1), Ordering::SeqCst);
    }

    fn open_duration_elapsed(&self) -> bool {
        let stored = self.open_since_nanos.load(Ordering::SeqCst);
        if stored == 0 {
            return false;
        }
        let elapsed_since_open = (self.base_instant.elapsed().as_nanos() as u64).saturating_sub(stored);
        elapsed_since_open >= self.config.open_duration.as_nanos() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.failure_threshold, 5);
        assert_eq!(cfg.success_threshold, 2);
        assert_eq!(cfg.open_duration, Duration::from_secs(30));
    }

    #[test]
    fn breaker_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<CircuitBreaker>();
    }

    fn test_config() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 3,
            quality_failure_threshold: 5,
            success_threshold: 2,
            open_duration: Duration::from_millis(100),
        }
    }

    #[tokio::test]
    async fn closed_passes_through_on_success() {
        let cb = CircuitBreaker::new(test_config());
        let result: Result<i32, CircuitBreakerError<&str>> = cb.call(|| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn closed_opens_after_threshold_failures() {
        let cb = CircuitBreaker::new(test_config());
        for _ in 0..3 {
            let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        }
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[tokio::test]
    async fn open_rejects_immediately() {
        let cb = CircuitBreaker::new(test_config());
        for _ in 0..3 {
            let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        }
        let result: Result<i32, CircuitBreakerError<&str>> = cb.call(|| async { Ok(42) }).await;
        assert!(matches!(result, Err(CircuitBreakerError::Open)));
    }

    #[tokio::test]
    async fn open_transitions_to_halfopen_after_duration() {
        tokio::time::pause();
        let cb = CircuitBreaker::new(test_config());
        for _ in 0..3 {
            let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        }
        assert_eq!(cb.state(), CircuitState::Open);

        tokio::time::advance(Duration::from_millis(150)).await;
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }

    #[tokio::test]
    async fn halfopen_closes_after_success_threshold() {
        tokio::time::pause();
        let cb = CircuitBreaker::new(test_config());
        // Trip to Open
        for _ in 0..3 {
            let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        }
        // Advance past open_duration
        tokio::time::advance(Duration::from_millis(150)).await;

        // success_threshold = 2
        let _: Result<i32, CircuitBreakerError<&str>> = cb.call(|| async { Ok(1) }).await;
        let _: Result<i32, CircuitBreakerError<&str>> = cb.call(|| async { Ok(2) }).await;
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn halfopen_reopens_on_failure() {
        tokio::time::pause();
        let cb = CircuitBreaker::new(test_config());
        for _ in 0..3 {
            let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        }
        tokio::time::advance(Duration::from_millis(150)).await;

        // Fail during half-open probe
        let _: Result<(), CircuitBreakerError<&str>> =
            cb.call(|| async { Err("fail again") }).await;
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn record_success_resets_failure_count() {
        let cb = CircuitBreaker::new(test_config());
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        // Should not trip even after one more failure
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn record_failure_trips_circuit() {
        let cb = CircuitBreaker::new(test_config());
        for _ in 0..3 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn reset_returns_to_closed() {
        let cb = CircuitBreaker::new(test_config());
        for _ in 0..3 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);
        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn success_in_closed_resets_failure_count() {
        let cb = CircuitBreaker::new(test_config());
        // 2 failures then a success should reset counter
        let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        let _: Result<i32, CircuitBreakerError<&str>> = cb.call(|| async { Ok(1) }).await;
        // Now only 1 more failure should not trip (need 3)
        let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn only_one_halfopen_probe_at_a_time() {
        tokio::time::pause();
        let cb = CircuitBreaker::new(test_config());
        for _ in 0..3 {
            let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("fail") }).await;
        }
        tokio::time::advance(Duration::from_millis(150)).await;
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // Simulate concurrent call: manually set probes counter
        use std::sync::atomic::Ordering;
        cb.half_open_probes.store(1, Ordering::SeqCst);

        // Second call should be rejected
        let result: Result<i32, CircuitBreakerError<&str>> = cb.call(|| async { Ok(42) }).await;
        assert!(matches!(result, Err(CircuitBreakerError::Open)));
    }
}
