use crate::client::{ChatContent, ChatMessageExt, ContentBlockInput, ToolDefinition};

/// Estimate token count from text using chars/4 heuristic.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() / 4) as u32
}

/// Estimate tokens for a single chat message (content + 4 overhead per message).
pub fn estimate_message_tokens(msg: &ChatMessageExt) -> u32 {
    let content_tokens = match &msg.content {
        ChatContent::Text(t) => estimate_tokens(t),
        ChatContent::Blocks(blocks) => blocks
            .iter()
            .map(|b| match b {
                ContentBlockInput::ToolResult { content, .. } => estimate_tokens(content),
            })
            .sum(),
    };
    content_tokens + 4
}

/// Estimate tokens for tool definitions by serializing to JSON.
pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> u32 {
    if tools.is_empty() {
        return 0;
    }
    let json = serde_json::to_string(tools).unwrap_or_default();
    estimate_tokens(&json)
}

/// Compute available token budget for conversation history.
///
/// Subtracts system prompt, tools, and output budget from the context window.
pub fn available_history_budget(
    context_window: u32,
    output_budget: u32,
    system_prompt: &str,
    tools: &[ToolDefinition],
) -> u32 {
    let system_tokens = estimate_tokens(system_prompt);
    let tools_tokens = estimate_tools_tokens(tools);
    context_window
        .saturating_sub(output_budget)
        .saturating_sub(system_tokens)
        .saturating_sub(tools_tokens)
}

/// Truncate history to fit within available tokens, dropping oldest messages first.
///
/// Returns (kept_messages, dropped_count).
pub fn truncate_history(
    messages: Vec<ChatMessageExt>,
    available_tokens: u32,
) -> (Vec<ChatMessageExt>, usize) {
    // Estimate total tokens
    let total: u32 = messages.iter().map(estimate_message_tokens).sum();
    if total <= available_tokens {
        return (messages, 0);
    }

    // Drop from the front until we fit
    let mut running = total;
    let mut drop_count = 0;
    for msg in &messages {
        if running <= available_tokens {
            break;
        }
        running -= estimate_message_tokens(msg);
        drop_count += 1;
    }

    let kept = messages.into_iter().skip(drop_count).collect();
    (kept, drop_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens(""), 0);
        // 12 chars / 4 = 3
        assert_eq!(estimate_tokens("hello world!"), 3);
        // 400 chars / 4 = 100
        let long = "a".repeat(400);
        assert_eq!(estimate_tokens(&long), 100);
    }

    #[test]
    fn estimate_message_tokens_text() {
        let msg = ChatMessageExt {
            role: "user".into(),
            content: ChatContent::Text("hello world!".into()),
        };
        // 3 content + 4 overhead = 7
        assert_eq!(estimate_message_tokens(&msg), 7);
    }

    #[test]
    fn estimate_message_tokens_blocks() {
        let msg = ChatMessageExt {
            role: "user".into(),
            content: ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                tool_use_id: "id1".into(),
                content: "a]".repeat(150), // 300 chars / 4 = 75
                is_error: false,
            }]),
        };
        assert_eq!(estimate_message_tokens(&msg), 75 + 4);
    }

    #[test]
    fn estimate_tools_tokens_empty() {
        assert_eq!(estimate_tools_tokens(&[]), 0);
    }

    #[test]
    fn estimate_tools_tokens_non_empty() {
        let tools = vec![ToolDefinition {
            name: "echo".into(),
            description: "echoes input".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let tokens = estimate_tools_tokens(&tools);
        assert!(tokens > 0);
    }

    #[test]
    fn available_history_budget_basic() {
        let budget = available_history_budget(200_000, 4096, "You are a bot.", &[]);
        // 200_000 - 4096 - (14/4=3) = 195_901
        assert_eq!(budget, 200_000 - 4096 - 3);
    }

    #[test]
    fn available_history_budget_saturates() {
        let budget = available_history_budget(100, 4096, "You are a bot.", &[]);
        assert_eq!(budget, 0);
    }

    #[test]
    fn truncate_history_no_truncation() {
        let messages = vec![
            ChatMessageExt {
                role: "user".into(),
                content: ChatContent::Text("hi".into()),
            },
            ChatMessageExt {
                role: "assistant".into(),
                content: ChatContent::Text("hello".into()),
            },
        ];
        let (kept, dropped) = truncate_history(messages.clone(), 100_000);
        assert_eq!(dropped, 0);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn truncate_history_drops_oldest() {
        let messages: Vec<ChatMessageExt> = (0..10)
            .map(|_| ChatMessageExt {
                role: "user".into(),
                content: ChatContent::Text("x".repeat(100)), // 25 + 4 = 29 tokens each
            })
            .collect();

        // Total: 10 * 29 = 290 tokens. Budget: 100 → need to drop some.
        let (kept, dropped) = truncate_history(messages, 100);
        assert!(dropped > 0);
        assert!(kept.len() < 10);
        // Verify remaining fits in budget
        let remaining_tokens: u32 = kept.iter().map(estimate_message_tokens).sum();
        assert!(remaining_tokens <= 100);
    }

    #[test]
    fn truncate_history_drops_all_if_needed() {
        let messages = vec![ChatMessageExt {
            role: "user".into(),
            content: ChatContent::Text("x".repeat(1000)),
        }];
        let (kept, dropped) = truncate_history(messages, 1);
        assert_eq!(dropped, 1);
        assert!(kept.is_empty());
    }
}
