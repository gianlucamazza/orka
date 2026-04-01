use std::{fmt::Write as _, sync::Arc};

use orka_llm::client::{ChatContent, ChatMessage, CompletionOptions, ContentBlockInput, LlmClient};
use tracing::warn;

use super::WorkspaceHandler;

impl WorkspaceHandler {
    /// Produce a plain-text transcript excerpt from a slice of messages.
    pub(super) fn build_transcript(messages: &[ChatMessage]) -> String {
        let mut transcript = String::new();
        for msg in messages {
            let text = match &msg.content {
                ChatContent::Text(t) => t.clone(),
                ChatContent::Blocks(blocks) => {
                    let mut parts = Vec::new();
                    for b in blocks {
                        match b {
                            ContentBlockInput::Text { text } => parts.push(text.clone()),
                            ContentBlockInput::ToolUse { name, .. } => {
                                parts.push(format!("[called {name}]"));
                            }
                            ContentBlockInput::ToolResult { content, .. } => {
                                // Keep tool results brief in the transcript.
                                let excerpt = if content.len() > 200 {
                                    format!("{}…", &content[..200])
                                } else {
                                    content.clone()
                                };
                                parts.push(format!("[result: {excerpt}]"));
                            }
                            _ => {}
                        }
                    }
                    if parts.is_empty() {
                        "[tool interaction]".to_string()
                    } else {
                        parts.join(" ")
                    }
                }
                _ => "[unsupported content]".to_string(),
            };
            let _ = writeln!(transcript, "{}: {}", msg.role, text);
        }
        transcript
    }

    /// Build a minimal summary from user-text messages when LLM summarisation
    /// is unavailable.
    pub(super) fn fallback_summary(messages: &[ChatMessage]) -> String {
        use orka_llm::client::Role;
        let bullets: Vec<String> = messages
            .iter()
            .filter(|m| m.role == Role::User)
            .filter_map(|m| match &m.content {
                ChatContent::Text(t) if !t.is_empty() => {
                    Some(format!("- {}", t.chars().take(120).collect::<String>()))
                }
                ChatContent::Blocks(blocks) => {
                    let text: String = blocks
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlockInput::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    if text.is_empty() {
                        None
                    } else {
                        Some(format!("- {}", text.chars().take(120).collect::<String>()))
                    }
                }
                _ => None,
            })
            .collect();

        if bullets.is_empty() {
            format!("[{} messages truncated]", messages.len())
        } else {
            format!(
                "Previous conversation (auto-summarized):\n{}",
                bullets.join("\n")
            )
        }
    }

    /// Summarise a slice of messages, optionally updating an existing rolling
    /// summary.
    ///
    /// When `existing_summary` is provided the LLM is asked to update it with
    /// the new turns, preserving user goals and unresolved tasks
    /// (incremental rolling pattern).
    pub(super) async fn summarize_messages(
        llm: &Arc<dyn LlmClient>,
        messages: &[ChatMessage],
        model: Option<&str>,
        existing_summary: Option<&str>,
    ) -> String {
        let transcript = Self::build_transcript(messages);

        let prompt_text = if let Some(old) = existing_summary {
            format!(
                "Update this existing summary with the new conversation turns. \
                 Preserve user goals, constraints, and unresolved tasks.\n\n\
                 Existing summary:\n{old}\n\nNew turns:\n{transcript}"
            )
        } else {
            format!(
                "Summarize the following conversation concisely, preserving \
                 key facts, decisions, and context:\n\n{transcript}"
            )
        };

        let summary_prompt = vec![ChatMessage::user(prompt_text)];

        let mut options = CompletionOptions::default();
        options.model = model.map(std::string::ToString::to_string);
        options.max_tokens = Some(1024);

        match llm
            .complete_with_options(
                summary_prompt,
                "You are a conversation summarizer. Be concise.",
                &options,
            )
            .await
        {
            Ok(summary) => summary,
            Err(e) => {
                warn!(%e, "failed to summarize conversation, using fallback");
                Self::fallback_summary(messages)
            }
        }
    }
}
