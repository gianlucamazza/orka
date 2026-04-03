use std::sync::OnceLock;

#[cfg(feature = "openai")]
use tiktoken_rs::CoreBPE;

use crate::client::{ChatContent, ChatMessage, ContentBlockInput, Role, ToolDefinition};

/// Model family hint for selecting the right tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerHint {
    /// `OpenAI` models (GPT-5, o3, o4-mini, etc.) — use `cl100k_base` /
    /// `o200k_base`.
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
            Some(m) if m.starts_with("gpt") || m.starts_with("o3") || m.starts_with("o4") => {
                Self::OpenAi
            }
            Some(m) if m.starts_with("claude") => Self::Anthropic,
            _ => Self::Unknown,
        }
    }
}

/// Thread-safe singleton for the `cl100k_base` tokenizer (used by GPT-series /
/// o-series).
#[cfg(feature = "openai")]
fn cl100k_bpe() -> &'static CoreBPE {
    static BPE: OnceLock<CoreBPE> = OnceLock::new();
    BPE.get_or_init(|| match tiktoken_rs::cl100k_base() {
        Ok(bpe) => bpe,
        Err(err) => panic!("failed to load cl100k_base tokenizer: {err}"),
    })
}

/// Estimate token count from text using a model-aware strategy.
///
/// - `OpenAI`: exact count via `cl100k_base` tokenizer (when the `openai`
///   feature is enabled) or chars/4 heuristic otherwise.
/// - Anthropic: chars / 3.5 (empirically closer than chars / 4).
/// - Unknown: chars / 4 heuristic.
pub fn estimate_tokens(text: &str) -> u32 {
    estimate_tokens_with_hint(text, TokenizerHint::Unknown)
}

/// Estimate token count using a specific tokenizer hint.
pub fn estimate_tokens_with_hint(text: &str, hint: TokenizerHint) -> u32 {
    match hint {
        TokenizerHint::OpenAi => {
            #[cfg(feature = "openai")]
            {
                cl100k_bpe().encode_ordinary(text).len() as u32
            }
            #[cfg(not(feature = "openai"))]
            {
                (text.len() / 4) as u32
            }
        }
        TokenizerHint::Anthropic => (text.len() as f64 / 3.5).ceil() as u32,
        TokenizerHint::Unknown => (text.len() / 4) as u32,
    }
}

/// Estimate tokens for a single chat message (content + 4 overhead per
/// message).
pub fn estimate_message_tokens(msg: &ChatMessage) -> u32 {
    estimate_message_tokens_with_hint(msg, TokenizerHint::Unknown)
}

/// Estimate message tokens using a specific tokenizer hint.
pub fn estimate_message_tokens_with_hint(msg: &ChatMessage, hint: TokenizerHint) -> u32 {
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
                // Thinking blocks are stripped before sending to the API (see anthropic.rs
                // build_ext_messages), so they must not count against the token budget.
                ContentBlockInput::Thinking { .. } | ContentBlockInput::Unknown => 0,
                // Images are treated as approximately 1000 tokens (rough estimate).
                ContentBlockInput::Image { .. } => 1000,
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

/// Returns true if `msg` starts a new logical conversation turn.
///
/// A turn-starting message is a user message with at least one `Text` block
/// (as opposed to a user message that only carries `ToolResult` blocks, which
/// is the API format for tool responses and belongs to the *previous* turn).
fn is_user_text_turn(msg: &ChatMessage) -> bool {
    if msg.role != Role::User {
        return false;
    }
    match &msg.content {
        ChatContent::Text(_) => true,
        ChatContent::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlockInput::Text { .. })),
    }
}

fn assistant_has_tool_use(msg: &ChatMessage, tool_use_id: &str) -> bool {
    if msg.role != Role::Assistant {
        return false;
    }
    match &msg.content {
        ChatContent::Blocks(blocks) => blocks.iter().any(|block| {
            matches!(
                block,
                ContentBlockInput::ToolUse { id, .. } if id == tool_use_id
            )
        }),
        ChatContent::Text(_) => false,
    }
}

/// Group `messages` into logical conversation turns.
///
/// A turn begins with a user text message and extends until (but not including)
/// the next user text message.  Tool-result user messages are part of the
/// *previous* turn because they belong to the assistant's last tool call.
///
/// Returns a `Vec<Range<usize>>`, one per turn.
pub fn group_into_turns(messages: &[ChatMessage]) -> Vec<std::ops::Range<usize>> {
    let mut turns = Vec::new();
    let mut turn_start: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        if is_user_text_turn(msg) {
            if let Some(start) = turn_start {
                turns.push(start..i);
            }
            turn_start = Some(i);
        }
    }
    if let Some(start) = turn_start {
        turns.push(start..messages.len());
    }
    turns
}

/// Remove orphaned `tool_result` blocks that no longer have a matching
/// assistant `tool_use` immediately before them, and strip `tool_use` blocks
/// with empty names from assistant messages (which some LLMs emit spuriously).
///
/// Returns (`sanitized_messages`, `removed_count`).
pub fn sanitize_tool_result_history(messages: Vec<ChatMessage>) -> (Vec<ChatMessage>, usize) {
    let mut sanitized = Vec::with_capacity(messages.len());
    let mut removed = 0usize;
    // IDs of empty-name ToolUse blocks stripped from the last assistant
    // message — used to also drop their matching ToolResult blocks.
    let mut empty_name_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for mut msg in messages {
        let ChatContent::Blocks(blocks) = &mut msg.content else {
            empty_name_ids.clear();
            sanitized.push(msg);
            continue;
        };

        if msg.role == Role::Assistant {
            // Strip ToolUse blocks with empty names; track their IDs so we
            // can drop the corresponding ToolResult blocks in the next User
            // message.
            empty_name_ids.clear();
            let original_len = blocks.len();
            blocks.retain(|block| {
                if let ContentBlockInput::ToolUse { id, name, .. } = block
                    && name.is_empty()
                {
                    empty_name_ids.insert(id.clone());
                    return false;
                }
                true
            });
            removed += original_len.saturating_sub(blocks.len());
            sanitized.push(msg);
            continue;
        }

        if msg.role != Role::User {
            empty_name_ids.clear();
            sanitized.push(msg);
            continue;
        }

        let prev_msg = sanitized.last();
        let original_len = blocks.len();
        let removed_ids = &empty_name_ids;
        blocks.retain(|block| match block {
            ContentBlockInput::ToolResult { tool_use_id, .. } => {
                // Drop results whose ToolUse was stripped (empty name) or has
                // no matching ToolUse in the preceding assistant message.
                !removed_ids.contains(tool_use_id.as_str())
                    && prev_msg.is_some_and(|prev| assistant_has_tool_use(prev, tool_use_id))
            }
            _ => true,
        });
        empty_name_ids.clear();

        removed += original_len.saturating_sub(blocks.len());

        if blocks.is_empty() {
            continue;
        }

        sanitized.push(msg);
    }

    (sanitized, removed)
}

/// Truncate history to fit within available tokens, dropping oldest turns
/// first.
///
/// When `protect_first_turn` is `true` the first conversation turn is never
/// dropped, regardless of budget pressure.
///
/// Returns (`kept_messages`, `dropped_count`).
pub fn truncate_history(
    messages: Vec<ChatMessage>,
    available_tokens: u32,
    protect_first_turn: bool,
) -> (Vec<ChatMessage>, usize) {
    truncate_history_with_hint(
        messages,
        available_tokens,
        TokenizerHint::Unknown,
        protect_first_turn,
    )
}

/// Truncate history using a specific tokenizer hint.
///
/// Drops whole conversation *turns* from the oldest end rather than individual
/// messages to avoid splitting assistant `tool_use` / user `tool_result` pairs.
/// The most-recent turn is always protected. When `protect_first_turn` is
/// `true`, the first turn is also never dropped.
pub fn truncate_history_with_hint(
    messages: Vec<ChatMessage>,
    available_tokens: u32,
    hint: TokenizerHint,
    protect_first_turn: bool,
) -> (Vec<ChatMessage>, usize) {
    let total: u32 = messages
        .iter()
        .map(|m| estimate_message_tokens_with_hint(m, hint))
        .sum();
    if total <= available_tokens {
        return (messages, 0);
    }

    let turns = group_into_turns(&messages);

    // Need at least two turns to drop anything while protecting the last turn.
    if turns.len() <= 1 {
        return (messages, 0);
    }

    let turn_tokens: Vec<u32> = turns
        .iter()
        .map(|r| {
            messages[r.clone()]
                .iter()
                .map(|m| estimate_message_tokens_with_hint(m, hint))
                .sum()
        })
        .collect();

    // Drop turns from the front, always protecting the last (current) turn.
    // When protect_first_turn is set, skip turn index 0.
    let mut running = total;
    let mut dropped_turn_indices: Vec<usize> = Vec::new();

    for (turn_idx, _turn_range) in turns[..turns.len() - 1].iter().enumerate() {
        if running <= available_tokens {
            break;
        }
        if protect_first_turn && turn_idx == 0 {
            continue;
        }
        running -= turn_tokens[turn_idx];
        dropped_turn_indices.push(turn_idx);
    }

    if dropped_turn_indices.is_empty() {
        return (messages, 0);
    }

    // Build the set of message indices that belong to dropped turns.
    let dropped_msg_indices: std::collections::HashSet<usize> = dropped_turn_indices
        .into_iter()
        .flat_map(|ti| turns[ti].clone())
        .collect();

    let drop_count = dropped_msg_indices.len();
    let kept = messages
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !dropped_msg_indices.contains(idx))
        .map(|(_, m)| m)
        .collect();
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
            TokenizerHint::from_model(Some("gpt-5-mini")),
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
        let msg = ChatMessage {
            role: Role::User,
            content: ChatContent::Text("hello world!".into()),
        };
        // 3 content + 4 overhead = 7
        assert_eq!(estimate_message_tokens(&msg), 7);
    }

    #[test]
    fn estimate_message_tokens_blocks() {
        let msg = ChatMessage {
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
            ChatMessage {
                role: Role::User,
                content: ChatContent::Text("hi".into()),
            },
            ChatMessage {
                role: Role::Assistant,
                content: ChatContent::Text("hello".into()),
            },
        ];
        let (kept, dropped) = truncate_history(messages, 100_000, false);
        assert_eq!(dropped, 0);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn truncate_history_drops_oldest() {
        // 10 user-text turns, each 29 tokens (25 content + 4 overhead).
        // The last turn is protected, so at most 9 can be dropped.
        let messages: Vec<ChatMessage> = (0..10)
            .map(|_| ChatMessage {
                role: Role::User,
                content: ChatContent::Text("x".repeat(100)), // 25 + 4 = 29 tokens each
            })
            .collect();

        // Total: 10 * 29 = 290 tokens. Budget: 100 -> need to drop some.
        let (kept, dropped) = truncate_history(messages, 100, false);
        assert!(dropped > 0);
        assert!(kept.len() < 10);
        // Verify remaining fits in budget
        let remaining_tokens: u32 = kept.iter().map(estimate_message_tokens).sum();
        assert!(remaining_tokens <= 100);
    }

    #[test]
    fn thinking_blocks_not_counted_in_token_estimate() {
        // Thinking blocks are stripped before sending to the API, so they must not
        // inflate the token count and cause unnecessary history truncation.
        let msg_with_thinking = ChatMessage {
            role: Role::Assistant,
            content: ChatContent::Blocks(vec![
                ContentBlockInput::Thinking {
                    thinking: "x".repeat(10_000),
                },
                ContentBlockInput::Text {
                    text: "hello".into(),
                },
            ]),
        };
        let msg_text_only = ChatMessage {
            role: Role::Assistant,
            content: ChatContent::Blocks(vec![ContentBlockInput::Text {
                text: "hello".into(),
            }]),
        };
        // Both messages should have the same token estimate: only the text block
        // counts.
        assert_eq!(
            estimate_message_tokens(&msg_with_thinking),
            estimate_message_tokens(&msg_text_only),
        );
    }

    #[test]
    fn truncate_history_single_turn_protected() {
        // A single turn (or single oversized message) is never dropped because
        // we must always preserve the current user request.
        let messages = vec![ChatMessage {
            role: Role::User,
            content: ChatContent::Text("x".repeat(1000)),
        }];
        let (kept, dropped) = truncate_history(messages, 1, false);
        assert_eq!(dropped, 0);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn truncate_history_first_turn_protected() {
        // When protect_first_turn is true, the first turn survives even under extreme
        // budget pressure.
        let mut messages: Vec<ChatMessage> = (0..6)
            .map(|_| ChatMessage {
                role: Role::User,
                content: ChatContent::Text("x".repeat(100)),
            })
            .collect();
        // Add a final turn so there are >1 turns.
        messages.push(ChatMessage {
            role: Role::User,
            content: ChatContent::Text("last request".into()),
        });

        let first_content = messages[0].content.clone();
        let (kept, _dropped) = truncate_history(messages, 50, true);
        // The first message must always be in the result.
        assert!(kept
            .iter()
            .any(|m| matches!((&m.content, &first_content), (ChatContent::Text(a), ChatContent::Text(b)) if a == b)));
    }

    #[test]
    fn group_into_turns_basic() {
        use crate::client::Role;
        let messages = vec![
            ChatMessage {
                role: Role::User,
                content: ChatContent::Text("hi".into()),
            },
            ChatMessage {
                role: Role::Assistant,
                content: ChatContent::Text("hello".into()),
            },
            ChatMessage {
                role: Role::User,
                content: ChatContent::Text("next".into()),
            },
        ];
        let turns = group_into_turns(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0], 0..2);
        assert_eq!(turns[1], 2..3);
    }

    #[test]
    fn sanitize_tool_result_history_keeps_valid_pairs() {
        let messages = vec![
            ChatMessage::new(
                Role::Assistant,
                ChatContent::Blocks(vec![ContentBlockInput::ToolUse {
                    id: "t1".into(),
                    name: "search".into(),
                    input: serde_json::json!({}),
                }]),
            ),
            ChatMessage::new(
                Role::User,
                ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "ok".into(),
                    is_error: false,
                }]),
            ),
        ];

        let (sanitized, removed) = sanitize_tool_result_history(messages);
        assert_eq!(removed, 0);
        assert_eq!(sanitized.len(), 2);
    }

    #[test]
    fn sanitize_tool_result_history_drops_orphan_result_message() {
        let messages = vec![ChatMessage::new(
            Role::User,
            ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                tool_use_id: "t1".into(),
                content: "orphan".into(),
                is_error: false,
            }]),
        )];

        let (sanitized, removed) = sanitize_tool_result_history(messages);
        assert_eq!(removed, 1);
        assert!(sanitized.is_empty());
    }

    #[test]
    fn sanitize_tool_result_history_drops_stale_replayed_result() {
        let messages = vec![
            ChatMessage::new(
                Role::Assistant,
                ChatContent::Blocks(vec![ContentBlockInput::ToolUse {
                    id: "t1".into(),
                    name: "search".into(),
                    input: serde_json::json!({}),
                }]),
            ),
            ChatMessage::new(
                Role::User,
                ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "ok".into(),
                    is_error: false,
                }]),
            ),
            ChatMessage::assistant("done"),
            ChatMessage::new(
                Role::User,
                ChatContent::Blocks(vec![ContentBlockInput::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "stale".into(),
                    is_error: false,
                }]),
            ),
        ];

        let (sanitized, removed) = sanitize_tool_result_history(messages);
        assert_eq!(removed, 1);
        assert_eq!(sanitized.len(), 3);
        assert!(matches!(sanitized[2].content, ChatContent::Text(ref t) if t == "done"));
    }

    #[test]
    fn sanitize_tool_result_history_preserves_non_tool_blocks_in_user_message() {
        let messages = vec![
            ChatMessage::assistant("done"),
            ChatMessage::new(
                Role::User,
                ChatContent::Blocks(vec![
                    ContentBlockInput::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "stale".into(),
                        is_error: false,
                    },
                    ContentBlockInput::Text {
                        text: "next question".into(),
                    },
                ]),
            ),
        ];

        let (sanitized, removed) = sanitize_tool_result_history(messages);
        assert_eq!(removed, 1);
        assert_eq!(sanitized.len(), 2);
        match &sanitized[1].content {
            ChatContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert!(matches!(blocks[0], ContentBlockInput::Text { .. }));
            }
            other => panic!("expected blocks, got {other:?}"),
        }
    }

    #[test]
    fn sanitize_strips_empty_name_tool_use_and_its_result() {
        // Simulate the real production bug: an assistant message with 4
        // empty-name ToolUse blocks (returned by gpt-5-mini) that survived in
        // the checkpoint, plus one valid ToolUse.
        let messages = vec![
            ChatMessage {
                role: Role::User,
                content: ChatContent::Text("hello".into()),
            },
            ChatMessage {
                role: Role::Assistant,
                content: ChatContent::Blocks(vec![
                    ContentBlockInput::ToolUse {
                        id: "call_empty1".into(),
                        name: String::new(), // empty name — should be stripped
                        input: serde_json::json!({}),
                    },
                    ContentBlockInput::ToolUse {
                        id: "call_valid1".into(),
                        name: "shell_exec".into(), // valid — should be kept
                        input: serde_json::json!({"cmd": "ls"}),
                    },
                ]),
            },
            ChatMessage {
                role: Role::User,
                content: ChatContent::Blocks(vec![
                    ContentBlockInput::ToolResult {
                        tool_use_id: "call_empty1".into(), // orphaned by empty-name removal
                        content: "result".into(),
                        is_error: false,
                    },
                    ContentBlockInput::ToolResult {
                        tool_use_id: "call_valid1".into(), // should be kept
                        content: "output".into(),
                        is_error: false,
                    },
                ]),
            },
        ];

        let (sanitized, removed) = sanitize_tool_result_history(messages);
        // 1 empty ToolUse + 1 matching ToolResult = 2 removed
        assert_eq!(removed, 2);
        assert_eq!(sanitized.len(), 3);

        // Assistant message keeps only the valid ToolUse
        match &sanitized[1].content {
            ChatContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert!(
                    matches!(blocks[0], ContentBlockInput::ToolUse { ref name, .. } if name == "shell_exec")
                );
            }
            other => panic!("expected blocks, got {other:?}"),
        }

        // User message keeps only the valid ToolResult
        match &sanitized[2].content {
            ChatContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert!(
                    matches!(blocks[0], ContentBlockInput::ToolResult { ref tool_use_id, .. } if tool_use_id == "call_valid1")
                );
            }
            other => panic!("expected blocks, got {other:?}"),
        }
    }
}
