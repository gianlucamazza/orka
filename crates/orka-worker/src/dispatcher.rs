//! Pluggable dispatch strategies for [`crate::WorkerPool`].
//!
//! [`HandlerDispatcher`] wraps an [`AgentHandler`] (used in tests and simple
//! deployments). [`GraphDispatcher`] drives the full agent graph with command
//! fast-path and conversation-history persistence.

use std::sync::Arc;

use async_trait::async_trait;
use orka_agent::{AgentGraph, ExecutionContext, GraphExecutor};
use orka_core::{
    CommandArgs, Envelope, OutboundMessage, Payload, Result, Session, traits::MemoryStore,
};
use orka_llm::client::ChatMessage;
use tracing::warn;

use crate::{commands::CommandRegistry, handler::AgentHandler, history};

/// Strategy for processing a single envelope after infrastructure concerns
/// (session load, locking, retry, tracing) are handled by
/// [`crate::WorkerPool`].
#[async_trait]
pub trait Dispatcher: Send + Sync + 'static {
    /// Process `envelope` in the context of `session`.
    ///
    /// Returns the outbound messages to publish, or an error that triggers the
    /// pool's retry/DLQ logic.
    async fn dispatch(
        &self,
        envelope: &Envelope,
        session: &Session,
    ) -> Result<Vec<OutboundMessage>>;
}

// ---------------------------------------------------------------------------
// HandlerDispatcher
// ---------------------------------------------------------------------------

/// [`Dispatcher`] that delegates to an [`AgentHandler`].
///
/// Used in tests and simple single-handler deployments.
pub struct HandlerDispatcher {
    handler: Arc<dyn AgentHandler>,
}

impl HandlerDispatcher {
    /// Create a new dispatcher wrapping `handler`.
    pub fn new(handler: Arc<dyn AgentHandler>) -> Self {
        Self { handler }
    }
}

#[async_trait]
impl Dispatcher for HandlerDispatcher {
    async fn dispatch(
        &self,
        envelope: &Envelope,
        session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        self.handler.handle(envelope, session).await
    }
}

// ---------------------------------------------------------------------------
// GraphDispatcher
// ---------------------------------------------------------------------------

/// [`Dispatcher`] that drives the agent graph with an optional command
/// fast-path and conversation-history persistence.
pub struct GraphDispatcher {
    executor: Arc<GraphExecutor>,
    graph: Arc<AgentGraph>,
    /// Memory store for loading and saving per-session conversation history.
    memory: Option<Arc<dyn MemoryStore>>,
    /// Command registry for slash-command fast-path (bypasses the LLM graph).
    commands: Option<Arc<CommandRegistry>>,
}

impl GraphDispatcher {
    /// Create a new dispatcher.
    pub fn new(
        executor: Arc<GraphExecutor>,
        graph: Arc<AgentGraph>,
        memory: Option<Arc<dyn MemoryStore>>,
        commands: Option<Arc<CommandRegistry>>,
    ) -> Self {
        Self {
            executor,
            graph,
            memory,
            commands,
        }
    }
}

#[async_trait]
impl Dispatcher for GraphDispatcher {
    async fn dispatch(
        &self,
        envelope: &Envelope,
        session: &Session,
    ) -> Result<Vec<OutboundMessage>> {
        // Command fast-path: execute registered commands without LLM
        if let Payload::Command(cmd) = &envelope.payload
            && let Some(ref registry) = self.commands
            && let Some(handler) = registry.get(&cmd.name)
        {
            let args = CommandArgs::from(cmd.clone());
            return handler.execute(&args, envelope, session).await;
        }

        let ctx = ExecutionContext::new(envelope.clone());

        // Load conversation history
        if let Some(ref mem) = self.memory {
            let history_key = format!("conversation:{}", envelope.session_id);
            match mem.recall(&history_key).await {
                Ok(Some(entry)) => {
                    let history: Vec<ChatMessage> =
                        serde_json::from_value(entry.value).unwrap_or_default();
                    if !history.is_empty() {
                        ctx.set_messages(history).await;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(%e, session_id = %envelope.session_id, "failed to load conversation history");
                }
            }
        }

        // Append current user message so the graph sees the live input
        let user_text = match &envelope.payload {
            Payload::Text(t) => Some(t.clone()),
            Payload::Media(m) => m
                .caption
                .clone()
                .or_else(|| Some(format!("[media: {}]", m.mime_type))),
            Payload::Command(c) => {
                let mut text = format!("/{}", c.name);
                if let Some(rest) = c.args.get("text").and_then(|v| v.as_str())
                    && !rest.is_empty()
                {
                    text.push(' ');
                    text.push_str(rest);
                }
                Some(text)
            }
            Payload::Event(_) | _ => None,
        };
        if let Some(text) = user_text {
            ctx.push_message(ChatMessage::user(text)).await;
        }

        // Execute graph
        let result = self.executor.execute(&self.graph, &ctx).await?;
        let outbound_msgs = result.into_outbound_messages(&ctx);

        // Persist updated history with compaction
        if let Some(ref mem) = self.memory {
            let history_key = format!("conversation:{}", envelope.session_id);
            let msgs = ctx.messages().await;
            history::save_history_compact(mem.as_ref(), &history_key, msgs).await;
        }

        Ok(outbound_msgs)
    }
}
