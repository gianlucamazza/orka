//! Shared conversation-history helpers used by both [`crate::WorkspaceHandler`]
//! and the graph path in [`crate::WorkerPoolGraph`].

use orka_core::MemoryEntry;
use orka_core::traits::MemoryStore;
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
                        let head = content[..half].to_string();
                        let tail = content[content.len() - half..].to_string();
                        let original_len = content.len();
                        *content = format!(
                            "{}\n... [truncated, original {original_len} chars] ...\n{}",
                            head, tail
                        );
                    }
                }
            }
            msg
        })
        .collect()
}

/// Persist a conversation history to the memory store with basic tool-result compaction.
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
