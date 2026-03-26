//! Outbound webhook delivery for A2A push notifications.

use std::{sync::Arc, time::Duration};

use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::{error::A2aError, push_store::PushNotificationStore, types::TaskEvent};

// ── WebhookDeliverer
// ──────────────────────────────────────────────────────────

/// Delivers [`TaskEvent`]s to registered webhook endpoints.
///
/// Uses a shared `reqwest::Client` for connection pooling. Each delivery
/// attempt POSTs the serialised event JSON to the registered URL with the
/// configured authentication header.  Failed attempts are retried up to
/// `max_retries` times with a fixed `retry_delay`.
pub struct WebhookDeliverer {
    client: reqwest::Client,
    push_store: Arc<dyn PushNotificationStore>,
    /// Maximum delivery attempts per event (first attempt + retries).
    pub max_retries: u32,
    /// Delay between retries.
    pub retry_delay: Duration,
}

impl WebhookDeliverer {
    /// Create a new deliverer with default retry settings (3 retries, 1 s
    /// delay).
    pub fn new(push_store: Arc<dyn PushNotificationStore>) -> Self {
        Self {
            client: reqwest::Client::new(),
            push_store,
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
        }
    }

    /// Attempt to deliver an event to the registered webhook for `task_id`.
    ///
    /// Does nothing if no config is registered for the task.
    /// Retries on network errors and 5xx responses.
    pub async fn deliver(&self, task_id: &str, event: &TaskEvent) -> Result<(), A2aError> {
        let Some(config) = self.push_store.get(task_id).await? else {
            return Ok(()); // no subscription registered
        };

        let body = serde_json::to_string(event)
            .map_err(|e| A2aError::Internal(format!("event serialization failed: {e}")))?;

        let mut attempts = 0u32;
        loop {
            let mut builder = self
                .client
                .post(&config.url)
                .header("Content-Type", "application/json")
                .body(body.clone());

            // Auth: explicit `authentication` takes precedence over `token`.
            if let Some(auth) = &config.authentication {
                let header_value = format!("{} {}", auth.scheme, auth.credentials);
                builder = builder.header("Authorization", header_value);
            } else if let Some(token) = &config.token {
                builder = builder.header("Authorization", format!("Bearer {token}"));
            }

            match builder.send().await {
                Ok(resp) if resp.status().is_success() => {
                    debug!(task_id, url = %config.url, "push notification delivered");
                    return Ok(());
                }
                Ok(resp) => {
                    let status = resp.status();
                    if attempts >= self.max_retries || !status.is_server_error() {
                        warn!(
                            task_id,
                            url = %config.url,
                            %status,
                            "push notification delivery failed (non-retryable or max retries)"
                        );
                        return Err(A2aError::Internal(format!(
                            "webhook delivery failed with status {status}"
                        )));
                    }
                }
                Err(e) => {
                    if attempts >= self.max_retries {
                        warn!(
                            task_id,
                            url = %config.url,
                            %e,
                            "push notification delivery failed after all retries"
                        );
                        return Err(A2aError::Internal(format!("webhook delivery error: {e}")));
                    }
                }
            }

            attempts += 1;
            tokio::time::sleep(self.retry_delay).await;
        }
    }
}

// ── Background delivery worker
// ────────────────────────────────────────────────

/// Returns `true` if this event is the last one in a task's stream.
fn is_final_event(event: &TaskEvent) -> bool {
    match event {
        TaskEvent::TaskStatusUpdate { is_final, .. }
        | TaskEvent::TaskArtifactUpdate { is_final, .. } => *is_final,
    }
}

/// Spawn a background task that delivers each broadcast event to the registered
/// webhook for `task_id`.
///
/// The worker exits when it receives a final event or when the broadcast
/// channel is closed (i.e. the task completed and all senders were dropped).
pub fn spawn_delivery_worker(
    rx: broadcast::Receiver<Arc<TaskEvent>>,
    deliverer: Arc<WebhookDeliverer>,
    task_id: String,
) {
    tokio::spawn(async move {
        let mut rx = rx;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let final_event = is_final_event(&event);
                    if let Err(e) = deliverer.deliver(&task_id, &event).await {
                        warn!(task_id, %e, "push notification delivery error");
                    }
                    if final_event {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(task_id, skipped = n, "push notification worker lagged");
                }
            }
        }
        debug!(task_id, "push notification delivery worker exiting");
    });
}
