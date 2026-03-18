use std::sync::OnceLock;

use tiktoken_rs::CoreBPE;

use crate::client::{ChatContent, ChatMessageExt, ContentBlockInput, ToolDefinition};

/// Model family hint for selecting the right tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerHint {
    /// OpenAI models (GPT-4, GPT-4o, etc.) — use cl100k_base / o200k_base.
    OpenAi,
    /// Anthropic models (Claude) — use chars/3.5 heuristic.
    Anthropic,
    /// Unknown / local models — use chars/4 heuristic.
    Unknown,
}

impl TokenizerHint {
    /// Infer the tokenizer hint from a model name.
    pub fn from_model(model: Option<&str>) -> Self {
        match model {
            Some(m) if m.starts_with("gpt") || m.starts_with("o1") || m.starts_with("o3") => {
                Self::OpenAi
            }
            Some(m) if m.starts_with("claude") => Self::Anthropic,
            _ => Self::Unknown,
        }
    }
}

/// Thread-safe singleton for the cl100k_base tokenizer (used by GPT-4 / GPT-4o).
fn cl100k_bpe() -> &'static CoreBPE {
    static BPE: OnceLock<CoreBPE> = OnceLock::new();
    BPE.get_or_init(|| tiktoken_rs::cl100k_base().expect("failed to load cl100k_base tokenizer"))
}

/// Estimate token count from text using a model-aware strategy.
///
/// - OpenAI: exact count via cl100k_base tokenizer.
/// - Anthropic: chars / 3.5 (empirically closer than chars / 4).
/// - Unknown: chars / 4 heuristic.
pub fn estimate_tokens(text: &str) -> u32 {
    estimate_tokens_with_hint(text, TokenizerHint::Unknown)
}

/// Estimate token count using a specific tokenizer hint.
pub fn estimate_tokens_with_hint(text: &str, hint: TokenizerHint) -> u32 {
    match hint {
        TokenizerHint::OpenAi => cl100k_bpe().encode_ordinary(text).len() as u32,
        TokenizerHint::Anthropic => (text.len() as f64 / 3.5).ceil() as u32,
        TokenizerHint::Unknown => (text.len() / 4) as u32,
    }
}

/// Estimate tokens for a single chat message (content + 4 overhead per message).
pub fn estimate_message_tokens(msg: &ChatMessageExt) -> u32 {
    estimate_message_tokens_with_hint(msg, TokenizerHint::Unknown)
}

/// Estimate message tokens using a specific tokenizer hint.
pub fn estimate_message_tokens_with_hint(msg: &ChatMessageExt, hint: TokenizerHint) -> u32 {
    let content_tokens = match &msg.content {
        ChatContent::Text(t) => estimate_tokens_with_hint(t, hint),
        ChatContent::Blocks(blocks) => blocks
            .iter()
            .map(|b| match b {
                ContentBlockInput::Text { text } => estimate_tokens_with_hint(text, hint),
                ContentBlockInput::ToolUse { input, .. } => estimate_tokens_with_hint(
                    &serde_json::to_string(input).unwrap_or_default(),
                    hint,
                ),
                ContentBlockInput::ToolResult { content, .. } => {
                    estimate_tokens_with_hint(content, hint)
                }
                ContentBlockInput::Unknown => 0,
            })
            .sum(),
    };
    content_tokens + 4
}

/// Estimate tokens for tool definitions by serializing to JSON.
pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> u32 {
    estimate_tools_tokens_with_hint(tools, TokenizerHint::Unknown)
}

/// Estimate tool definition tokens using a specific tokenizer hint.
pub fn estimate_tools_tokens_with_hint(tools: &[ToolDefinition], hint: TokenizerHint) -> u32 {
    if tools.is_empty() {
        return 0;
    }
    let json = serde_json::to_string(tools).unwrap_or_default();
    estimate_tokens_with_hint(&json, hint)
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
    available_history_budget_with_hint(
        context_window,
        output_budget,
        system_prompt,
        tools,
        TokenizerHint::Unknown,
    )
}

/// Compute available token budget using a specific tokenizer hint.
pub fn available_history_budget_with_hint(
    context_window: u32,
    output_budget: u32,
    system_prompt: &str,
    tools: &[ToolDefinition],
    hint: TokenizerHint,
) -> u32 {
    let system_tokens = estimate_tokens_with_hint(system_prompt, hint);
    let tools_tokens = estimate_tools_tokens_with_hint(tools, hint);
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
    truncate_history_with_hint(messages, available_tokens, TokenizerHint::Unknown)
}

/// Truncate history using a specific tokenizer hint.
pub fn truncate_history_with_hint(
    messages: Vec<ChatMessageExt>,
    available_tokens: u32,
    hint: TokenizerHint,
) -> (Vec<ChatMessageExt>, usize) {
    let total: u32 = messages
        .iter()
        .map(|m| estimate_message_tokens_with_hint(m, hint))
        .sum();
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
        running -= estimate_message_tokens_with_hint(msg, hint);
        drop_count += 1;
    }

    let kept = messages.into_iter().skip(drop_count).collect();
    (kept, drop_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Role;

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
    fn estimate_tokens_openai_uses_tokenizer() {
        let tokens = estimate_tokens_with_hint("Hello, world!", TokenizerHint::OpenAi);
        // cl100k_base should give a precise count (4 tokens for "Hello, world!")
        assert!(tokens > 0);
        assert!(tokens < 10);
    }

    #[test]
    fn estimate_tokens_anthropic_uses_3_5() {
        // 35 chars / 3.5 = 10
        let text = "a".repeat(35);
        assert_eq!(
            estimate_tokens_with_hint(&text, TokenizerHint::Anthropic),
            10
        );
    }

    #[test]
    fn tokenizer_hint_from_model() {
        assert_eq!(
            TokenizerHint::from_model(Some("gpt-4o")),
            TokenizerHint::OpenAi
        );
        assert_eq!(
            TokenizerHint::from_model(Some("claude-sonnet-4-6")),
            TokenizerHint::Anthropic
        );
        assert_eq!(
            TokenizerHint::from_model(Some("llama-3")),
            TokenizerHint::Unknown
        );
        assert_eq!(TokenizerHint::from_model(None), TokenizerHint::Unknown);
    }

    #[test]
    fn estimate_message_tokens_text() {
        let msg = ChatMessageExt {
            role: Role::User,
            content: ChatContent::Text("hello world!".into()),
        };
        // 3 content + 4 overhead = 7
        assert_eq!(estimate_message_tokens(&msg), 7);
    }

    #[test]
    fn estimate_message_tokens_blocks() {
        let msg = ChatMessageExt {
            role: Role::User,
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
                role: Role::User,
                content: ChatContent::Text("hi".into()),
            },
            ChatMessageExt {
                role: Role::Assistant,
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
                role: Role::User,
                content: ChatContent::Text("x".repeat(100)), // 25 + 4 = 29 tokens each
            })
            .collect();

        // Total: 10 * 29 = 290 tokens. Budget: 100 -> need to drop some.
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
            role: Role::User,
            content: ChatContent::Text("x".repeat(1000)),
        }];
        let (kept, dropped) = truncate_history(messages, 1);
        assert_eq!(dropped, 1);
        assert!(kept.is_empty());
    }
}
