//! Axum route handlers for the A2A JSON-RPC protocol.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, sse::Event},
};
use chrono::Utc;
use orka_core::{SkillContext, SkillInput, traits::SecretManager};
use orka_skills::SkillRegistry;
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use tracing::debug;
use uuid::Uuid;

use crate::{
    error::A2aError,
    jsonrpc::{JsonRpcResponse, parse_request},
    push_store::PushNotificationStore,
    store::TaskStore,
    types::{
        AgentCard, Artifact, ListTasksParams, Message, MessageKind, Part, PushNotificationConfig,
        Role, Task, TaskEvent, TaskKind, TaskState, TaskStatus,
    },
    webhook::{WebhookDeliverer, spawn_delivery_worker},
};

// ── Shared state
// ──────────────────────────────────────────────────────────────

/// Shared state injected into A2A axum handlers.
#[derive(Clone)]
pub struct A2aState {
    /// Agent card served at `GET /.well-known/agent.json`.
    pub agent_card: AgentCard,
    /// Skill registry for routing requests.
    pub skills: Arc<SkillRegistry>,
    /// Secret manager for skill execution contexts.
    pub secrets: Arc<dyn SecretManager>,
    /// Task persistence backend.
    pub task_store: Arc<dyn TaskStore>,
    /// Live broadcast channels, one per in-flight task (keyed by task ID).
    pub task_events: Arc<Mutex<HashMap<String, broadcast::Sender<Arc<TaskEvent>>>>>,
    /// Push notification config backend.
    pub push_store: Arc<dyn PushNotificationStore>,
    /// Outbound webhook deliverer for push notifications.
    pub webhook_deliverer: Arc<WebhookDeliverer>,
}

// ── Helpers
// ───────────────────────────────────────────────────────────────────

/// Return the current UTC time as an ISO 8601 string with millisecond
/// precision.
fn now_ts() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Extract the first plain-text part from an A2A message JSON value.
pub fn extract_text_from_message(message: &Value) -> String {
    if let Some(parts) = message["parts"].as_array() {
        for part in parts {
            if part["kind"].as_str() == Some("text")
                && let Some(text) = part["text"].as_str()
            {
                return text.to_string();
            }
        }
    }
    String::new()
}

/// Find a skill for the given message text:
/// 1. Exact lookup by `skillId` param if present.
/// 2. Fallback: longest skill name that matches `text` as a whole word (i.e.
///    text equals the name, or the name is followed by a space). Using
///    longest-match prevents "echo" from shadowing "echoplus".
fn find_skill_name(skills: &SkillRegistry, text: &str, skill_id: Option<&str>) -> Option<String> {
    if let Some(id) = skill_id
        && skills.get(id).is_some()
    {
        return Some(id.to_string());
    }
    let mut candidates: Vec<String> = skills
        .list()
        .into_iter()
        .filter(|name| {
            text.strip_prefix(name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
        .map(str::to_owned)
        .collect();
    // Prefer longer matches so more-specific skill names win.
    candidates.sort_by_key(|n| std::cmp::Reverse(n.len()));
    candidates.into_iter().next()
}

/// Build a new [`Message`] with `Role::User` from raw text.
fn user_message(text: &str, context_id: &str, task_id: &str) -> Message {
    Message {
        kind: MessageKind::Message,
        role: Role::User,
        parts: vec![Part::Text {
            text: text.to_string(),
            metadata: None,
        }],
        message_id: Uuid::now_v7().to_string(),
        context_id: Some(context_id.to_string()),
        task_id: Some(task_id.to_string()),
        metadata: None,
    }
}

/// Build a new [`Message`] with `Role::Agent` from raw text.
fn agent_message(text: &str, context_id: &str, task_id: &str) -> Message {
    Message {
        kind: MessageKind::Message,
        role: Role::Agent,
        parts: vec![Part::Text {
            text: text.to_string(),
            metadata: None,
        }],
        message_id: Uuid::now_v7().to_string(),
        context_id: Some(context_id.to_string()),
        task_id: Some(task_id.to_string()),
        metadata: None,
    }
}

// ── Handlers
// ──────────────────────────────────────────────────────────────────

/// GET /.well-known/agent.json
#[utoipa::path(
    get,
    path = "/.well-known/agent.json",
    responses(
        (status = 200, description = "Agent discovery card", body = crate::types::AgentCard)
    ),
    tag = "a2a"
)]
pub async fn handle_agent_card(State(state): State<A2aState>) -> impl IntoResponse {
    Json(state.agent_card)
}

/// POST /a2a — JSON-RPC 2.0 dispatcher (A2A v1.0).
#[utoipa::path(
    post,
    path = "/a2a",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "JSON-RPC 2.0 response", body = serde_json::Value)
    ),
    tag = "a2a"
)]
pub async fn handle_a2a(
    State(state): State<A2aState>,
    Json(raw): Json<Value>,
) -> axum::response::Response {
    let request = match parse_request(raw) {
        Ok(r) => r,
        Err(err_response) => {
            return Json(serde_json::to_value(err_response).unwrap_or_default()).into_response();
        }
    };

    let id = request.id.clone();

    // Record method and tenant for distributed tracing.
    tracing::debug!(method = %request.method, tenant = ?request.tenant, "a2a_rpc");

    let tenant = request.tenant.clone();

    // Streaming methods return SSE directly.
    match request.method.as_str() {
        "message/stream" => {
            return handle_message_stream(&state, &request.params, tenant).await;
        }
        "tasks/subscribe" | "tasks/resubscribe" => {
            return handle_tasks_subscribe(&state, &request.params).await;
        }
        _ => {}
    }

    let result: Result<Value, A2aError> = match request.method.as_str() {
        "message/send" => handle_message_send(&state, &request.params, tenant).await,

        "tasks/get" => handle_task_get(&state, &request.params).await,
        "tasks/list" => handle_task_list(&state, &request.params).await,
        "tasks/cancel" => handle_task_cancel(&state, &request.params).await,

        "tasks/pushNotificationConfig/set" => {
            handle_push_notification_set(&state, &request.params).await
        }
        "tasks/pushNotificationConfig/get" => {
            handle_push_notification_get(&state, &request.params).await
        }
        "tasks/pushNotificationConfig/list" => {
            handle_push_notification_list(&state, &request.params).await
        }
        "tasks/pushNotificationConfig/delete" => {
            handle_push_notification_delete(&state, &request.params).await
        }

        "agentCard/getExtended" => handle_get_extended_agent_card(&state),

        method => Err(A2aError::MethodNotFound(method.to_string())),
    };

    let response = match result {
        Ok(data) => JsonRpcResponse::ok(id, data),
        Err(ref err) => JsonRpcResponse::from_error(id, err),
    };

    Json(serde_json::to_value(response).unwrap_or_default()).into_response()
}

// ── Method handlers
// ───────────────────────────────────────────────────────────

/// `message/send` — create a task, invoke a matching skill, return the task.
async fn handle_message_send(
    state: &A2aState,
    params: &Value,
    tenant: Option<String>,
) -> Result<Value, A2aError> {
    let task_id = params["id"]
        .as_str()
        .map_or_else(|| Uuid::now_v7().to_string(), String::from);

    let context_id = params["contextId"]
        .as_str()
        .map_or_else(|| Uuid::now_v7().to_string(), String::from);

    let message = params
        .get("message")
        .ok_or_else(|| A2aError::InvalidParams("missing 'message' parameter".into()))?;

    let text = extract_text_from_message(message);
    let skill_id = params["skillId"].as_str();
    let now = now_ts();

    let submitted = TaskStatus {
        state: TaskState::Submitted,
        message: None,
        timestamp: now.clone(),
    };
    let working = TaskStatus {
        state: TaskState::Working,
        message: Some(user_message(&text, &context_id, &task_id)),
        timestamp: now.clone(),
    };

    let metadata = tenant
        .as_deref()
        .map(|t| HashMap::from([("tenant".to_string(), Value::String(t.to_string()))]));

    let mut task = Task {
        kind: TaskKind::Task,
        id: task_id.clone(),
        context_id: context_id.clone(),
        history: vec![submitted],
        status: working,
        artifacts: Vec::new(),
        created_at: Utc::now(),
        last_modified: Utc::now(),
        metadata,
    };

    state.task_store.put(task.clone()).await?;

    let response_text = if let Some(skill_name) = find_skill_name(&state.skills, &text, skill_id) {
        let args = HashMap::from([("input".to_string(), Value::String(text.clone()))]);
        let input =
            SkillInput::new(args).with_context(SkillContext::new(state.secrets.clone(), None));

        match state.skills.invoke(&skill_name, input).await {
            Ok(output) => output.data.to_string(),
            Err(e) => {
                let prev = std::mem::replace(
                    &mut task.status,
                    TaskStatus {
                        state: TaskState::Failed,
                        message: None,
                        timestamp: now_ts(),
                    },
                );
                task.history.push(prev);
                task.last_modified = Utc::now();
                state.task_store.put(task).await?;
                return Err(A2aError::Internal(e.to_string()));
            }
        }
    } else {
        format!("Received: {text}")
    };

    let agent_msg = agent_message(&response_text, &context_id, &task_id);
    let artifact = Artifact {
        artifact_id: Uuid::now_v7().to_string(),
        parts: vec![Part::Text {
            text: response_text,
            metadata: None,
        }],
        name: Some("response".to_string()),
        description: None,
        metadata: None,
    };

    let prev = std::mem::replace(
        &mut task.status,
        TaskStatus {
            state: TaskState::Completed,
            message: Some(agent_msg),
            timestamp: now_ts(),
        },
    );
    task.history.push(prev);
    task.artifacts.push(artifact);
    task.last_modified = Utc::now();

    debug!(task_id, "A2A task completed");
    let status_for_push = task.status.clone();
    let json = serde_json::to_value(&task).map_err(|e| A2aError::Internal(e.to_string()))?;
    state.task_store.put(task).await?;
    maybe_deliver_push(state, &task_id, &context_id, &status_for_push).await;
    Ok(json)
}

/// Fire-and-forget push notification for a task status update if a config is
/// registered for the given task.
async fn maybe_deliver_push(
    state: &A2aState,
    task_id: &str,
    context_id: &str,
    status: &TaskStatus,
) {
    if let Ok(Some(_)) = state.push_store.get(task_id).await {
        let event = Arc::new(TaskEvent::TaskStatusUpdate {
            task_id: task_id.to_string(),
            context_id: context_id.to_string(),
            status: status.clone(),
            is_final: true,
        });
        let deliverer = state.webhook_deliverer.clone();
        let tid = task_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = deliverer.deliver(&tid, &event).await {
                debug!(task_id = %tid, %e, "push notification delivery failed");
            }
        });
    }
}

/// `message/stream` — create a task and stream events over SSE.
async fn handle_message_stream(
    state: &A2aState,
    params: &Value,
    tenant: Option<String>,
) -> axum::response::Response {
    use axum::response::sse::Sse;
    use tokio_stream::{StreamExt, wrappers::BroadcastStream};

    let task_id = params
        .get("id")
        .and_then(|v| v.as_str())
        .map_or_else(|| Uuid::now_v7().to_string(), String::from);

    let context_id = params
        .get("contextId")
        .and_then(|v| v.as_str())
        .map_or_else(|| Uuid::now_v7().to_string(), String::from);

    let message = if let Some(m) = params.get("message") {
        m.clone()
    } else {
        let err = A2aError::InvalidParams("missing 'message' parameter".into());
        return Json(
            serde_json::to_value(JsonRpcResponse::from_error(None, &err)).unwrap_or_default(),
        )
        .into_response();
    };

    let text = extract_text_from_message(&message);
    let skill_id = params["skillId"].as_str().map(String::from);
    let now = now_ts();

    let (tx, rx) = broadcast::channel::<Arc<TaskEvent>>(64);
    state
        .task_events
        .lock()
        .await
        .insert(task_id.clone(), tx.clone());

    let metadata = tenant
        .as_deref()
        .map(|t| HashMap::from([("tenant".to_string(), Value::String(t.to_string()))]));
    let task = Task {
        kind: TaskKind::Task,
        id: task_id.clone(),
        context_id: context_id.clone(),
        status: TaskStatus {
            state: TaskState::Working,
            message: Some(user_message(&text, &context_id, &task_id)),
            timestamp: now.clone(),
        },
        history: vec![TaskStatus {
            state: TaskState::Submitted,
            message: None,
            timestamp: now,
        }],
        artifacts: Vec::new(),
        created_at: Utc::now(),
        last_modified: Utc::now(),
        metadata,
    };

    if let Err(e) = state.task_store.put(task).await {
        state.task_events.lock().await.remove(&task_id);
        let err = A2aError::Internal(e.to_string());
        return Json(
            serde_json::to_value(JsonRpcResponse::from_error(None, &err)).unwrap_or_default(),
        )
        .into_response();
    }

    let state_bg = state.clone();
    tokio::spawn(async move {
        invoke_and_stream(state_bg, task_id, context_id, text, skill_id.as_deref(), tx).await;
    });

    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let data = serde_json::to_string(event.as_ref()).unwrap_or_default();
            Some(Ok::<Event, std::convert::Infallible>(
                Event::default().data(data),
            ))
        }
        Err(_) => None,
    });

    Sse::new(stream).into_response()
}

/// Background worker: invoke a skill and broadcast events to SSE subscribers
/// and push notification delivery workers.
async fn invoke_and_stream(
    state: A2aState,
    task_id: String,
    context_id: String,
    text: String,
    skill_id: Option<&str>,
    tx: broadcast::Sender<Arc<TaskEvent>>,
) {
    // Spawn push delivery worker if a config is registered for this task.
    if state
        .push_store
        .get(&task_id)
        .await
        .is_ok_and(|c| c.is_some())
    {
        spawn_delivery_worker(
            tx.subscribe(),
            state.webhook_deliverer.clone(),
            task_id.clone(),
        );
    }

    // Announce working state.
    let _ = tx.send(Arc::new(TaskEvent::TaskStatusUpdate {
        task_id: task_id.clone(),
        context_id: context_id.clone(),
        status: TaskStatus {
            state: TaskState::Working,
            message: Some(user_message(&text, &context_id, &task_id)),
            timestamp: now_ts(),
        },
        is_final: false,
    }));

    let response_text = if let Some(skill_name) = find_skill_name(&state.skills, &text, skill_id) {
        let args = HashMap::from([("input".to_string(), Value::String(text.clone()))]);
        let input =
            SkillInput::new(args).with_context(SkillContext::new(state.secrets.clone(), None));

        match state.skills.invoke(&skill_name, input).await {
            Ok(output) => output.data.to_string(),
            Err(e) => {
                let failed_status = TaskStatus {
                    state: TaskState::Failed,
                    message: None,
                    timestamp: now_ts(),
                };
                if let Ok(Some(mut task)) = state.task_store.get(&task_id).await {
                    let prev = std::mem::replace(&mut task.status, failed_status.clone());
                    task.history.push(prev);
                    task.last_modified = Utc::now();
                    let _ = state.task_store.put(task).await;
                }
                let _ = tx.send(Arc::new(TaskEvent::TaskStatusUpdate {
                    task_id: task_id.clone(),
                    context_id: context_id.clone(),
                    status: failed_status,
                    is_final: true,
                }));
                state.task_events.lock().await.remove(&task_id);
                debug!(%e, task_id, "A2A streaming task failed");
                return;
            }
        }
    } else {
        format!("Received: {text}")
    };

    let agent_msg = agent_message(&response_text, &context_id, &task_id);
    let artifact = Artifact {
        artifact_id: Uuid::now_v7().to_string(),
        parts: vec![Part::Text {
            text: response_text.clone(),
            metadata: None,
        }],
        name: Some("response".to_string()),
        description: None,
        metadata: None,
    };

    let _ = tx.send(Arc::new(TaskEvent::TaskArtifactUpdate {
        task_id: task_id.clone(),
        context_id: context_id.clone(),
        artifact: artifact.clone(),
        last_chunk: true,
        is_final: false,
    }));

    let completed_status = TaskStatus {
        state: TaskState::Completed,
        message: Some(agent_msg),
        timestamp: now_ts(),
    };

    if let Ok(Some(mut task)) = state.task_store.get(&task_id).await {
        let prev = std::mem::replace(&mut task.status, completed_status.clone());
        task.history.push(prev);
        task.artifacts.push(artifact);
        task.last_modified = Utc::now();
        let _ = state.task_store.put(task).await;
    }

    let _ = tx.send(Arc::new(TaskEvent::TaskStatusUpdate {
        task_id: task_id.clone(),
        context_id: context_id.clone(),
        status: completed_status,
        is_final: true,
    }));

    state.task_events.lock().await.remove(&task_id);
    debug!(task_id, "A2A streaming task completed");
}

/// `tasks/get` — retrieve a task by ID.
async fn handle_task_get(state: &A2aState, params: &Value) -> Result<Value, A2aError> {
    let task_id = params["id"]
        .as_str()
        .ok_or_else(|| A2aError::InvalidParams("missing 'id' parameter".into()))?;

    match state.task_store.get(task_id).await? {
        Some(task) => serde_json::to_value(task).map_err(|e| A2aError::Internal(e.to_string())),
        None => Err(A2aError::TaskNotFound),
    }
}

/// `tasks/list` — list tasks with optional state filtering and pagination.
async fn handle_task_list(state: &A2aState, params: &Value) -> Result<Value, A2aError> {
    let p: ListTasksParams = if params.is_null() || params == &json!({}) {
        ListTasksParams::default()
    } else {
        serde_json::from_value(params.clone())
            .map_err(|e| A2aError::InvalidParams(e.to_string()))?
    };

    let result = state.task_store.list(&p).await?;
    serde_json::to_value(&result).map_err(|e| A2aError::Internal(e.to_string()))
}

/// `tasks/cancel` — transition a task to the `Canceled` state.
async fn handle_task_cancel(state: &A2aState, params: &Value) -> Result<Value, A2aError> {
    let task_id = params["id"]
        .as_str()
        .ok_or_else(|| A2aError::InvalidParams("missing 'id' parameter".into()))?;

    match state.task_store.get(task_id).await? {
        Some(mut task) => {
            if task.status.state.is_terminal() {
                return Err(A2aError::TaskNotCancelable);
            }
            let prev = std::mem::replace(
                &mut task.status,
                TaskStatus {
                    state: TaskState::Canceled,
                    message: None,
                    timestamp: now_ts(),
                },
            );
            task.history.push(prev);
            task.last_modified = Utc::now();
            state.task_store.put(task.clone()).await?;
            serde_json::to_value(&task).map_err(|e| A2aError::Internal(e.to_string()))
        }
        None => Err(A2aError::TaskNotFound),
    }
}

/// `tasks/subscribe` — subscribe to live SSE events for a task.
async fn handle_tasks_subscribe(state: &A2aState, params: &Value) -> axum::response::Response {
    use axum::response::sse::Sse;
    use tokio_stream::{StreamExt, wrappers::BroadcastStream};

    let task_id = if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
        id.to_string()
    } else {
        let err = A2aError::InvalidParams("missing 'id' parameter".into());
        return Json(
            serde_json::to_value(JsonRpcResponse::from_error(None, &err)).unwrap_or_default(),
        )
        .into_response();
    };

    let live_rx = state
        .task_events
        .lock()
        .await
        .get(&task_id)
        .map(tokio::sync::broadcast::Sender::subscribe);

    if let Some(rx) = live_rx {
        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(event) => {
                let data = serde_json::to_string(event.as_ref()).unwrap_or_default();
                Some(Ok::<Event, std::convert::Infallible>(
                    Event::default().data(data),
                ))
            }
            Err(_) => None,
        });
        return Sse::new(stream).into_response();
    }

    match state.task_store.get(&task_id).await {
        Ok(Some(task)) => {
            let event = Arc::new(TaskEvent::TaskStatusUpdate {
                task_id: task.id.clone(),
                context_id: task.context_id.clone(),
                status: task.status,
                is_final: true,
            });
            let data = serde_json::to_string(&*event).unwrap_or_default();
            let stream = tokio_stream::iter(std::iter::once(
                Ok::<Event, std::convert::Infallible>(Event::default().data(data)),
            ));
            Sse::new(stream).into_response()
        }
        Ok(None) => Json(
            serde_json::to_value(JsonRpcResponse::from_error(None, &A2aError::TaskNotFound))
                .unwrap_or_default(),
        )
        .into_response(),
        Err(e) => Json(
            serde_json::to_value(JsonRpcResponse::from_error(
                None,
                &A2aError::Internal(e.to_string()),
            ))
            .unwrap_or_default(),
        )
        .into_response(),
    }
}

/// `tasks/pushNotificationConfig/set` — register a webhook for task events.
async fn handle_push_notification_set(state: &A2aState, params: &Value) -> Result<Value, A2aError> {
    let config: PushNotificationConfig = serde_json::from_value(params.clone())
        .map_err(|e| A2aError::InvalidParams(format!("invalid push notification config: {e}")))?;

    let task_id = config.task_id.clone();
    state.push_store.set(config.clone()).await?;

    // If the task is currently in-flight, attach a delivery worker immediately.
    if let Some(tx) = state.task_events.lock().await.get(&task_id) {
        spawn_delivery_worker(
            tx.subscribe(),
            state.webhook_deliverer.clone(),
            task_id.clone(),
        );
    } else if let Ok(Some(task)) = state.task_store.get(&task_id).await {
        // Task already completed — deliver the terminal status once.
        let event = TaskEvent::TaskStatusUpdate {
            task_id: task.id.clone(),
            context_id: task.context_id.clone(),
            status: task.status,
            is_final: true,
        };
        let deliverer = state.webhook_deliverer.clone();
        tokio::spawn(async move {
            if let Err(e) = deliverer.deliver(&task_id, &event).await {
                debug!(%e, task_id, "push notification delivery for completed task failed");
            }
        });
    }

    serde_json::to_value(&config).map_err(|e| A2aError::Internal(e.to_string()))
}

/// `tasks/pushNotificationConfig/get` — retrieve the webhook config for a task.
async fn handle_push_notification_get(state: &A2aState, params: &Value) -> Result<Value, A2aError> {
    let task_id = params["taskId"]
        .as_str()
        .ok_or_else(|| A2aError::InvalidParams("missing 'taskId' parameter".into()))?;

    match state.push_store.get(task_id).await? {
        Some(config) => {
            serde_json::to_value(&config).map_err(|e| A2aError::Internal(e.to_string()))
        }
        None => Err(A2aError::TaskNotFound),
    }
}

/// `tasks/pushNotificationConfig/list` — list all registered webhook configs.
async fn handle_push_notification_list(
    state: &A2aState,
    _params: &Value,
) -> Result<Value, A2aError> {
    let configs = state.push_store.list().await?;
    serde_json::to_value(configs).map_err(|e| A2aError::Internal(e.to_string()))
}

/// `tasks/pushNotificationConfig/delete` — remove the webhook config for a
/// task.
async fn handle_push_notification_delete(
    state: &A2aState,
    params: &Value,
) -> Result<Value, A2aError> {
    let task_id = params["taskId"]
        .as_str()
        .ok_or_else(|| A2aError::InvalidParams("missing 'taskId' parameter".into()))?;

    let existed = state.push_store.delete(task_id).await?;
    if existed {
        Ok(json!({"deleted": true}))
    } else {
        Err(A2aError::TaskNotFound)
    }
}

/// `agentCard/getExtended` — return the full agent card.
///
/// When auth is enabled on this route, only authenticated callers reach this
/// handler; the `AuthLayer` rejects unauthenticated requests with 401 before
/// this function is invoked.
fn handle_get_extended_agent_card(state: &A2aState) -> Result<Value, A2aError> {
    serde_json::to_value(&state.agent_card).map_err(|e| A2aError::Internal(e.to_string()))
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Build an axum `Router` with all A2A routes (both public and private).
pub fn a2a_router(state: A2aState) -> axum::Router {
    let (public, protected) = a2a_routes_split(state);
    public.merge(protected)
}

/// Split A2A routes into public and protected halves.
///
/// - **public**: `GET /.well-known/agent.json` — always unauthenticated
///   (discovery).
/// - **protected**: `POST /a2a` — can be wrapped with an `AuthLayer` when
///   `a2a.auth_enabled` is true in the server config.
///
/// Use [`a2a_router`] when no auth splitting is needed.
pub fn a2a_routes_split(state: A2aState) -> (axum::Router, axum::Router) {
    let public = axum::Router::new()
        .route(
            "/.well-known/agent.json",
            axum::routing::get(handle_agent_card),
        )
        .with_state(state.clone());
    let protected = axum::Router::new()
        .route("/a2a", axum::routing::post(handle_a2a))
        .with_state(state);
    (public, protected)
}
