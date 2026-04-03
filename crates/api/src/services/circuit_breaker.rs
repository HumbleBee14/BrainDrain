use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::error::AppError;

/// Async circuit breaker for protecting calls to external services.
///
/// State machine: Closed → Open → HalfOpen → Closed (on success) or Open (on failure).
///
/// - **Closed:** Requests pass through. Consecutive failures increment a counter.
///   When failures reach `failure_threshold`, the circuit trips to Open.
/// - **Open:** All requests are rejected immediately with `ServiceUnavailable`.
///   After `recovery_timeout`, transitions to HalfOpen.
/// - **HalfOpen:** One probe request is allowed. If it succeeds, reset to Closed.
///   If it fails, return to Open. Concurrent requests during probe are rejected.
#[derive(Clone)]
pub struct CircuitBreaker {
    state: Arc<Mutex<CircuitState>>,
    failure_threshold: u32,
    recovery_timeout: Duration,
}

enum CircuitState {
    Closed {
        consecutive_failures: u32,
    },
    Open {
        tripped_at: Instant,
    },
    /// A probe request is in flight. All other requests are rejected.
    HalfOpen,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, recovery_timeout: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(CircuitState::Closed {
                consecutive_failures: 0,
            })),
            failure_threshold,
            recovery_timeout,
        }
    }

    /// Execute an async operation through the circuit breaker.
    ///
    /// Returns `AppError::ServiceUnavailable` when the circuit is open.
    pub async fn execute<F, Fut, T>(&self, f: F) -> Result<T, AppError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        // Check if we should allow the request
        {
            let mut state = self.state.lock().await;
            match *state {
                CircuitState::Closed { .. } => {
                    // Allow request — drop lock so concurrent Closed requests proceed
                }
                CircuitState::Open { tripped_at } => {
                    if tripped_at.elapsed() >= self.recovery_timeout {
                        // Transition to HalfOpen: allow exactly one probe request.
                        // The state is set to HalfOpen while the lock is held, so
                        // concurrent requests will see HalfOpen and be rejected.
                        *state = CircuitState::HalfOpen;
                        tracing::info!("Circuit breaker transitioning to half-open");
                    } else {
                        return Err(AppError::ServiceUnavailable {
                            message: "Inference service temporarily unavailable".to_string(),
                        });
                    }
                }
                CircuitState::HalfOpen => {
                    // Already probing — reject concurrent requests while probing
                    return Err(AppError::ServiceUnavailable {
                        message: "Inference service temporarily unavailable".to_string(),
                    });
                }
            }
        }

        // Execute the request (lock is dropped so concurrent Closed requests proceed)
        let result = f().await;

        // Update state based on result
        {
            let mut state = self.state.lock().await;
            match result {
                Ok(_) => {
                    if !matches!(
                        *state,
                        CircuitState::Closed {
                            consecutive_failures: 0
                        }
                    ) {
                        tracing::info!("Circuit breaker reset to closed");
                    }
                    *state = CircuitState::Closed {
                        consecutive_failures: 0,
                    };
                }
                Err(_) => match *state {
                    CircuitState::Closed {
                        consecutive_failures,
                    } => {
                        let new_count = consecutive_failures + 1;
                        if new_count >= self.failure_threshold {
                            tracing::warn!(failures = new_count, "Circuit breaker tripped to open");
                            *state = CircuitState::Open {
                                tripped_at: Instant::now(),
                            };
                        } else {
                            *state = CircuitState::Closed {
                                consecutive_failures: new_count,
                            };
                        }
                    }
                    CircuitState::HalfOpen => {
                        tracing::warn!("Circuit breaker probe failed, returning to open");
                        *state = CircuitState::Open {
                            tripped_at: Instant::now(),
                        };
                    }
                    CircuitState::Open { .. } => {
                        // Shouldn't happen, but keep as open
                    }
                },
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_allows_requests() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(1));
        let result: Result<i32, AppError> = cb.execute(|| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn trips_after_threshold_failures() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));

        for _ in 0..3 {
            let _ = cb
                .execute(|| async { Err::<(), _>(AppError::Internal(anyhow::anyhow!("fail"))) })
                .await;
        }

        // Next call should be rejected immediately
        let result = cb.execute(|| async { Ok::<(), AppError>(()) }).await;
        assert!(matches!(result, Err(AppError::ServiceUnavailable { .. })));
    }

    #[tokio::test]
    async fn success_resets_failure_count() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));

        // 2 failures (under threshold)
        for _ in 0..2 {
            let _ = cb
                .execute(|| async { Err::<(), _>(AppError::Internal(anyhow::anyhow!("fail"))) })
                .await;
        }

        // 1 success resets
        let _ = cb.execute(|| async { Ok::<(), AppError>(()) }).await;

        // 2 more failures should not trip (count reset)
        for _ in 0..2 {
            let _ = cb
                .execute(|| async { Err::<(), _>(AppError::Internal(anyhow::anyhow!("fail"))) })
                .await;
        }

        // Should still be closed
        let result = cb.execute(|| async { Ok::<i32, AppError>(1) }).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn recovers_after_timeout() {
        let cb = CircuitBreaker::new(1, Duration::from_millis(50));

        // Trip it
        let _ = cb
            .execute(|| async { Err::<(), _>(AppError::Internal(anyhow::anyhow!("fail"))) })
            .await;

        // Should be open
        let result = cb.execute(|| async { Ok::<(), AppError>(()) }).await;
        assert!(matches!(result, Err(AppError::ServiceUnavailable { .. })));

        // Wait for recovery timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Should transition to half-open and allow probe
        let result = cb.execute(|| async { Ok::<i32, AppError>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn half_open_failure_returns_to_open() {
        let cb = CircuitBreaker::new(1, Duration::from_millis(50));

        // Trip it
        let _ = cb
            .execute(|| async { Err::<(), _>(AppError::Internal(anyhow::anyhow!("fail"))) })
            .await;

        // Wait for recovery
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Probe fails
        let _ = cb
            .execute(|| async {
                Err::<(), _>(AppError::Internal(anyhow::anyhow!("still failing")))
            })
            .await;

        // Should be back to open
        let result = cb.execute(|| async { Ok::<(), AppError>(()) }).await;
        assert!(matches!(result, Err(AppError::ServiceUnavailable { .. })));
    }
}
