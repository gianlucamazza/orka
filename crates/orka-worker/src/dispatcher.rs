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
            let history = history::load_graph_history(mem.as_ref(), &history_key).await;
            if !history.is_empty() {
                ctx.set_messages(history).await;
            }
        }

        // Append current user message so the graph sees the live input
        match &envelope.payload {
            Payload::Text(t) => {
                ctx.push_message(ChatMessage::user(t.clone())).await;
            }
            Payload::RichInput(input) => {
                // Build a single user turn that preserves text plus image
                // attachments when possible, and degrades non-image files to
                // textual placeholders.
                use orka_llm::client::{ChatContent, ContentBlockInput, ImageSource};
                let mut blocks = Vec::new();
                let mut fallback_lines = Vec::new();

                if let Some(text) = input.text.clone()
                    && !text.trim().is_empty()
                {
                    blocks.push(ContentBlockInput::Text {
                        text: text.clone(),
                    });
                    fallback_lines.push(text);
                }

                for attachment in &input.attachments {
                    if attachment.mime_type.starts_with("image/") {
                        if let Some(data) = attachment.data_base64.clone() {
                            blocks.push(ContentBlockInput::Image {
                                source: ImageSource::Base64 {
                                    media_type: attachment.mime_type.clone(),
                                    data,
                                },
                            });
                        } else if !attachment.url.is_empty() {
                            blocks.push(ContentBlockInput::Image {
                                source: ImageSource::Url {
                                    url: attachment.url.clone(),
                                },
                            });
                        }
                        if let Some(caption) = attachment.caption.clone()
                            && !caption.is_empty()
                        {
                            blocks.push(ContentBlockInput::Text { text: caption });
                        }
                    } else {
                        let label = attachment
                            .caption
                            .clone()
                            .or_else(|| attachment.filename.clone())
                            .unwrap_or_else(|| format!("[attachment: {}]", attachment.mime_type));
                        fallback_lines.push(label);
                    }
                }

                if !blocks.is_empty() {
                    ctx.push_message(ChatMessage::new(
                        orka_llm::client::Role::User,
                        ChatContent::Blocks(blocks),
                    ))
                    .await;
                } else if !fallback_lines.is_empty() {
                    ctx.push_message(ChatMessage::user(fallback_lines.join("\n"))).await;
                }
            }
            Payload::Media(m)
                if m.mime_type.starts_with("image/")
                    && (!m.url.is_empty() || m.data_base64.is_some()) =>
            {
                use orka_llm::client::{ChatContent, ContentBlockInput, ImageSource};
                let source = if let Some(data) = m.data_base64.clone() {
                    ImageSource::Base64 {
                        media_type: m.mime_type.clone(),
                        data,
                    }
                } else {
                    ImageSource::Url { url: m.url.clone() }
                };
                let mut blocks = vec![ContentBlockInput::Image { source }];
                if let Some(ref caption) = m.caption
                    && !caption.is_empty()
                {
                    blocks.push(ContentBlockInput::Text {
                        text: caption.clone(),
                    });
                }
                ctx.push_message(ChatMessage::new(
                    orka_llm::client::Role::User,
                    ChatContent::Blocks(blocks),
                ))
                .await;
            }
            Payload::Media(m) => {
                let text = m
                    .caption
                    .clone()
                    .or_else(|| m.filename.clone())
                    .unwrap_or_else(|| format!("[media: {}]", m.mime_type));
                ctx.push_message(ChatMessage::user(text)).await;
            }
            Payload::Command(c) => {
                let mut text = format!("/{}", c.name);
                if let Some(rest) = c.args.get("text").and_then(|v| v.as_str())
                    && !rest.is_empty()
                {
                    text.push(' ');
                    text.push_str(rest);
                }
                ctx.push_message(ChatMessage::user(text)).await;
            }
            Payload::Event(_) | _ => {}
        }

        // Execute graph — or resume if this envelope carries a resume token.
        let result = if let Some(resume_run_id) = envelope.metadata.get("__resume_run_id")
            && let Some(run_id) = resume_run_id.as_str()
        {
            match self.executor.resume(run_id, &self.graph).await? {
                Some(r) => r,
                // No checkpoint or already at terminal state — treat as
                // completed with empty response.
                None => orka_agent::ExecutionResult {
                    response: String::new(),
                    attachments: vec![],
                    agents_executed: vec![],
                    total_iterations: 0,
                    total_tokens: 0,
                    duration_ms: 0,
                    stop_reason: orka_core::stream::AgentStopReason::Complete,
                },
            }
        } else {
            self.executor.execute(&self.graph, &ctx).await?
        };
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
