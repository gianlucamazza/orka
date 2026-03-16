use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CircuitState {
    Closed = 0,
    Open = 1,
    HalfOpen = 2,
}

impl CircuitState {
    fn from_u8(v: u8) -> Self {
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
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// Number of consecutive successes in half-open state before closing.
    pub success_threshold: u32,
    /// How long the circuit stays open before transitioning to half-open.
    pub open_duration: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            open_duration: Duration::from_secs(30),
        }
    }
}

/// A generic circuit breaker that wraps fallible async operations.
///
/// The breaker is `Send + Sync + 'static` and can be shared across tasks
/// via `Arc`.
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    /// 0 = Closed, 1 = Open, 2 = HalfOpen
    state: AtomicU8,
    failure_count: AtomicU32,
    success_count: AtomicU32,
    half_open_probes: AtomicU32,
    /// The instant when the circuit transitioned to Open.
    open_since: Mutex<Option<Instant>>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: AtomicU8::new(CircuitState::Closed as u8),
            failure_count: AtomicU32::new(0),
            success_count: AtomicU32::new(0),
            half_open_probes: AtomicU32::new(0),
            open_since: Mutex::new(None),
        }
    }

    /// Execute `f` through the circuit breaker.
    ///
    /// - **Closed**: calls `f`. On failure, increments the failure counter.
    ///   If the threshold is reached the circuit opens.
    /// - **Open**: if `open_duration` has elapsed, transitions to half-open
    ///   and allows the call. Otherwise returns [`CircuitBreakerError::Open`].
    /// - **HalfOpen**: allows at most one concurrent probe. On success the
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
        if raw == CircuitState::Open as u8 {
            if self.open_duration_elapsed() {
                self.state
                    .store(CircuitState::HalfOpen as u8, Ordering::SeqCst);
                self.half_open_probes.store(0, Ordering::SeqCst);
                return CircuitState::HalfOpen;
            }
        }

        CircuitState::from_u8(raw)
    }

    /// Manually reset the circuit breaker to the Closed state.
    pub fn reset(&self) {
        self.state
            .store(CircuitState::Closed as u8, Ordering::SeqCst);
        self.failure_count.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        self.half_open_probes.store(0, Ordering::SeqCst);
        *self.open_since.lock().unwrap() = None;
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
        *self.open_since.lock().unwrap() = Some(Instant::now());
    }

    fn open_duration_elapsed(&self) -> bool {
        let guard = self.open_since.lock().unwrap();
        match *guard {
            Some(since) => since.elapsed() >= self.config.open_duration,
            None => false,
        }
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
}
