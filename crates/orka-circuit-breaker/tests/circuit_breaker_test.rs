use std::time::Duration;

use orka_circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerError, CircuitState,
};

#[derive(Debug, Clone, PartialEq)]
struct TestError(String);

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn test_config() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        failure_threshold: 3,
        quality_failure_threshold: 5,
        success_threshold: 2,
        open_duration: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn closed_state_allows_calls() {
    let cb = CircuitBreaker::new(test_config());
    let result: Result<i32, CircuitBreakerError<TestError>> = cb.call(|| async { Ok(42) }).await;
    assert_eq!(result.unwrap(), 42);
    assert_eq!(cb.state(), CircuitState::Closed);
}

#[tokio::test]
async fn transitions_to_open_after_n_failures() {
    let cb = CircuitBreaker::new(test_config());

    for i in 0..3 {
        let result: Result<i32, CircuitBreakerError<TestError>> = cb
            .call(|| async { Err(TestError(format!("fail {i}"))) })
            .await;
        assert!(matches!(result, Err(CircuitBreakerError::Inner(_))));
    }

    assert_eq!(cb.state(), CircuitState::Open);
}

#[tokio::test]
async fn open_state_rejects_immediately() {
    let cb = CircuitBreaker::new(test_config());

    // Trip the breaker.
    for _ in 0..3 {
        let _: Result<i32, _> = cb
            .call(|| async { Err::<i32, _>(TestError("fail".into())) })
            .await;
    }

    assert_eq!(cb.state(), CircuitState::Open);

    let result: Result<i32, CircuitBreakerError<TestError>> = cb.call(|| async { Ok(1) }).await;
    assert!(matches!(result, Err(CircuitBreakerError::Open)));
}

#[tokio::test(start_paused = true)]
async fn transitions_to_half_open_after_duration() {
    let cb = CircuitBreaker::new(test_config());

    // Trip the breaker.
    for _ in 0..3 {
        let _: Result<i32, _> = cb
            .call(|| async { Err::<i32, _>(TestError("fail".into())) })
            .await;
    }
    assert_eq!(cb.state(), CircuitState::Open);

    // Advance past the open_duration.
    tokio::time::advance(Duration::from_secs(6)).await;

    assert_eq!(cb.state(), CircuitState::HalfOpen);
}

#[tokio::test(start_paused = true)]
async fn half_open_success_closes_circuit() {
    let cfg = CircuitBreakerConfig {
        failure_threshold: 3,
        quality_failure_threshold: 5,
        success_threshold: 2,
        open_duration: Duration::from_secs(5),
    };
    let cb = CircuitBreaker::new(cfg);

    // Trip the breaker.
    for _ in 0..3 {
        let _: Result<i32, _> = cb
            .call(|| async { Err::<i32, _>(TestError("fail".into())) })
            .await;
    }
    assert_eq!(cb.state(), CircuitState::Open);

    tokio::time::advance(Duration::from_secs(6)).await;

    // First successful probe.
    let result: Result<i32, CircuitBreakerError<TestError>> = cb.call(|| async { Ok(1) }).await;
    assert!(result.is_ok());

    // Need to advance time again to allow another probe after the open->half-open transition
    // triggered by the call above. The breaker is still in HalfOpen waiting for success_threshold.
    // But we need the probe counter to allow another call. Since half_open allows 1 concurrent
    // and we already finished, the counter is back to 0.

    // Wait for open_duration again since the first successful call might have been done
    // via the Open->HalfOpen path.
    tokio::time::advance(Duration::from_secs(6)).await;

    // Second successful probe should close the circuit.
    let result: Result<i32, CircuitBreakerError<TestError>> = cb.call(|| async { Ok(2) }).await;
    assert!(result.is_ok());
    assert_eq!(cb.state(), CircuitState::Closed);
}

#[tokio::test(start_paused = true)]
async fn half_open_failure_reopens_circuit() {
    let cb = CircuitBreaker::new(test_config());

    // Trip the breaker.
    for _ in 0..3 {
        let _: Result<i32, _> = cb
            .call(|| async { Err::<i32, _>(TestError("fail".into())) })
            .await;
    }

    tokio::time::advance(Duration::from_secs(6)).await;
    assert_eq!(cb.state(), CircuitState::HalfOpen);

    // Fail in half-open.
    let result: Result<i32, CircuitBreakerError<TestError>> = cb
        .call(|| async { Err(TestError("half-open fail".into())) })
        .await;
    assert!(matches!(result, Err(CircuitBreakerError::Inner(_))));
    assert_eq!(cb.state(), CircuitState::Open);
}

#[tokio::test]
async fn manual_reset_works() {
    let cb = CircuitBreaker::new(test_config());

    // Trip the breaker.
    for _ in 0..3 {
        let _: Result<i32, _> = cb
            .call(|| async { Err::<i32, _>(TestError("fail".into())) })
            .await;
    }
    assert_eq!(cb.state(), CircuitState::Open);

    cb.reset();
    assert_eq!(cb.state(), CircuitState::Closed);

    // Should be able to make calls again.
    let result: Result<i32, CircuitBreakerError<TestError>> = cb.call(|| async { Ok(99) }).await;
    assert_eq!(result.unwrap(), 99);
}

#[test]
fn config_defaults_are_sane() {
    let cfg = CircuitBreakerConfig::default();
    assert_eq!(cfg.failure_threshold, 5);
    assert_eq!(cfg.success_threshold, 2);
    assert_eq!(cfg.open_duration, Duration::from_secs(30));
}

#[tokio::test]
async fn success_resets_failure_count() {
    let cb = CircuitBreaker::new(test_config());

    // Two failures (below threshold of 3).
    for _ in 0..2 {
        let _: Result<i32, _> = cb
            .call(|| async { Err::<i32, _>(TestError("fail".into())) })
            .await;
    }

    // One success resets the counter.
    let _: Result<i32, CircuitBreakerError<TestError>> = cb.call(|| async { Ok(1) }).await;

    // Two more failures should NOT open (counter was reset).
    for _ in 0..2 {
        let _: Result<i32, _> = cb
            .call(|| async { Err::<i32, _>(TestError("fail".into())) })
            .await;
    }
    assert_eq!(cb.state(), CircuitState::Closed);
}
