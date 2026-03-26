//! Shared conversation-history helpers used by both [`crate::WorkspaceHandler`]
//! and the graph path in [`crate::WorkerPoolGraph`].

use orka_core::{MemoryEntry, traits::MemoryStore};
use orka_llm::client::{ChatContent, ChatMessage, ContentBlockInput};
use tracing::warn;

/// Compact oversized tool results in a message list.
///
/// Tool results longer than `max_chars` are replaced with a head+tail excerpt
/// to reduce storage size without losing user-visible context.
pub fn compact_tool_results(messages: Vec<ChatMessage>, max_chars: usize) -> Vec<ChatMessage> {
    messages
        .into_iter()
        .map(|mut msg| {
            if let ChatContent::Blocks(ref mut blocks) = msg.content {
                for block in blocks.iter_mut() {
                    if let ContentBlockInput::ToolResult { content, .. } = block
                        && content.len() > max_chars
                    {
                        let half = max_chars / 2;
                        let head_end = content.floor_char_boundary(half);
                        let tail_start =
                            content.ceil_char_boundary(content.len().saturating_sub(half));
                        let head = &content[..head_end];
                        let tail = &content[tail_start..];
                        let original_len = content.len();
                        *content = format!(
                            "{head}\n... [truncated, original {original_len} chars] ...\n{tail}"
                        );
                    }
                }
            }
            msg
        })
        .collect()
}

/// Persist a conversation history to the memory store with basic tool-result
/// compaction.
///
/// Used by the graph execution path, which does not run the full summarization
/// pipeline available in [`crate::WorkspaceHandler`].
pub async fn save_history_compact(
    memory: &dyn MemoryStore,
    history_key: &str,
    messages: Vec<ChatMessage>,
) {
    const MAX_TOOL_RESULT_CHARS: usize = 2000;
    let messages = compact_tool_results(messages, MAX_TOOL_RESULT_CHARS);

    if messages.is_empty() {
        return;
    }

    match serde_json::to_value(&messages) {
        Ok(v) => {
            let entry =
                MemoryEntry::new(history_key, v).with_tags(vec!["conversation".to_string()]);
            if let Err(e) = memory.store(history_key, entry, None).await {
                warn!(%e, key = %history_key, "failed to persist conversation history");
            }
        }
        Err(e) => {
            warn!(%e, key = %history_key, "failed to serialize conversation history");
        }
    }
}

#[cfg(test)]
mod tests {
    use orka_llm::client::{ChatContent, ChatMessage, ContentBlockInput, Role};

    use super::*;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage::user(text)
    }

    fn tool_result_msg(content: &str) -> ChatMessage {
        ChatMessage::new(
            Role::User,
            ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                tool_use_id: "t1".into(),
                content: content.into(),
                is_error: false,
            }]),
        )
    }

    #[test]
    fn compact_tool_results_no_truncation() {
        let msgs = vec![tool_result_msg("short result")];
        let result = compact_tool_results(msgs, 1000);
        if let ChatContent::Blocks(ref blocks) = result[0].content
            && let ContentBlockInput::ToolResult { content, .. } = &blocks[0]
        {
            assert_eq!(content, "short result");
        } else {
            panic!("expected ToolResult block");
        }
    }

    #[test]
    fn compact_tool_results_truncates_long_result() {
        let long = "x".repeat(5000);
        let msgs = vec![tool_result_msg(&long)];
        let result = compact_tool_results(msgs, 100);
        if let ChatContent::Blocks(ref blocks) = result[0].content
            && let ContentBlockInput::ToolResult { content, .. } = &blocks[0]
        {
            assert!(content.contains("[truncated"));
            assert!(content.contains("5000"));
            assert!(content.len() < long.len());
        } else {
            panic!("expected ToolResult block");
        }
    }

    #[test]
    fn compact_tool_results_preserves_non_tool_messages() {
        let msgs = vec![user_msg("hello"), tool_result_msg("data")];
        let result = compact_tool_results(msgs, 1000);
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0].content, ChatContent::Text(ref t) if t == "hello"));
    }

    #[test]
    fn compact_tool_results_empty_vec() {
        let result = compact_tool_results(vec![], 100);
        assert!(result.is_empty());
    }
}
