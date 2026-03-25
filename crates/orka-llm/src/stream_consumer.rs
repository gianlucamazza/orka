//! Streaming LLM response consumer — bridges an [`LlmToolStream`] into the
//! core [`StreamRegistry`] and reconstructs a [`CompletionResponse`].

use futures_util::StreamExt;
use orka_core::{
    MessageId, Result, SessionId,
    stream::{StreamChunk, StreamChunkKind, StreamRegistry},
};
use tracing::warn;

use crate::client::{
    CompletionResponse, ContentBlock, LlmToolStream, StreamEvent, ToolCall, Usage,
};

/// Consume a streaming LLM response, emitting [`StreamChunk`]s to the
/// registry and reconstructing a [`CompletionResponse`] from the events.
pub async fn consume_stream(
    mut stream: LlmToolStream,
    session_id: &SessionId,
    stream_registry: &StreamRegistry,
    channel: &str,
    reply_to: Option<&MessageId>,
) -> Result<CompletionResponse> {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut current_tool_id: Option<String> = None;
    let mut current_tool_name: Option<String> = None;
    let mut current_tool_input = String::new();
    let mut usage = Usage::default();
    let mut stop_reason = None;

    while let Some(event) = stream.next().await {
        let event = event?;
        match event {
            StreamEvent::ThinkingDelta(delta) => {
                stream_registry.send(StreamChunk::new(
                    *session_id,
                    channel.to_string(),
                    reply_to.copied(),
                    StreamChunkKind::ThinkingDelta(delta.clone()),
                ));
                thinking.push_str(&delta);
            }
            StreamEvent::TextDelta(delta) => {
                stream_registry.send(StreamChunk::new(
                    *session_id,
                    channel.to_string(),
                    reply_to.copied(),
                    StreamChunkKind::Delta(delta.clone()),
                ));
                text.push_str(&delta);
            }
            StreamEvent::ToolUseStart { id, name } => {
                stream_registry.send(StreamChunk::new(
                    *session_id,
                    channel.to_string(),
                    reply_to.copied(),
                    StreamChunkKind::ToolStart {
                        name: name.clone(),
                        id: id.clone(),
                    },
                ));
                current_tool_id = Some(id);
                current_tool_name = Some(name);
                current_tool_input.clear();
            }
            StreamEvent::ToolUseInputDelta(delta) => {
                current_tool_input.push_str(&delta);
            }
            StreamEvent::ToolUseEnd { id, input } => {
                let name = current_tool_name.take().unwrap_or_default();
                let final_input = if input != serde_json::Value::Null {
                    input
                } else {
                    serde_json::from_str(&current_tool_input).unwrap_or_else(|e| {
                        warn!(%e, tool = %name, "malformed tool input JSON, using empty object");
                        serde_json::Value::Object(Default::default())
                    })
                };
                stream_registry.send(StreamChunk::new(
                    *session_id,
                    channel.to_string(),
                    reply_to.copied(),
                    StreamChunkKind::ToolEnd {
                        id: id.clone(),
                        success: true,
                    },
                ));
                tool_calls.push(ToolCall::new(id, name, final_input));
                current_tool_id = None;
                current_tool_input.clear();
            }
            StreamEvent::Usage(u) => usage = u,
            StreamEvent::Stop(reason) => stop_reason = Some(reason),
        }
    }

    // If we were mid-tool when the stream ended, treat it as incomplete.
    if let Some(id) = current_tool_id {
        stream_registry.send(StreamChunk::new(
            *session_id,
            channel.to_string(),
            reply_to.copied(),
            StreamChunkKind::ToolEnd { id, success: false },
        ));
    }

    let mut blocks = Vec::new();
    if !thinking.is_empty() {
        blocks.push(ContentBlock::Thinking(thinking));
    }
    if !text.is_empty() {
        blocks.push(ContentBlock::Text(text));
    }
    for call in tool_calls {
        blocks.push(ContentBlock::ToolUse(call));
    }

    Ok(CompletionResponse::new(blocks, usage, stop_reason))
}

#[cfg(test)]
mod tests {
    use orka_core::{SessionId, stream::StreamRegistry};

    use super::*;
    use crate::client::{ContentBlock, StopReason, StreamEvent};

    fn make_stream(events: Vec<StreamEvent>) -> LlmToolStream {
        let stream =
            futures_util::stream::iter(events.into_iter().map(Ok::<StreamEvent, orka_core::Error>));
        Box::pin(stream)
    }

    #[tokio::test]
    async fn consumes_text_stream() {
        let events = vec![
            StreamEvent::TextDelta("hello".into()),
            StreamEvent::TextDelta(" world".into()),
            StreamEvent::Stop(StopReason::EndTurn),
        ];
        let registry = StreamRegistry::new();
        let session_id = SessionId::new();

        let resp = consume_stream(make_stream(events), &session_id, &registry, "ch", None)
            .await
            .unwrap();

        assert_eq!(resp.blocks.len(), 1);
        assert!(matches!(&resp.blocks[0], ContentBlock::Text(t) if t == "hello world"));
        assert_eq!(resp.stop_reason, Some(StopReason::EndTurn));
    }

    #[tokio::test]
    async fn consumes_tool_use_stream() {
        let events = vec![
            StreamEvent::ToolUseStart {
                id: "t1".into(),
                name: "echo".into(),
            },
            StreamEvent::ToolUseInputDelta(r#"{"msg":"hi"}"#.into()),
            StreamEvent::ToolUseEnd {
                id: "t1".into(),
                input: serde_json::Value::Null, // parsed from accumulated input
            },
            StreamEvent::Stop(StopReason::ToolUse),
        ];
        let registry = StreamRegistry::new();
        let session_id = SessionId::new();

        let resp = consume_stream(make_stream(events), &session_id, &registry, "ch", None)
            .await
            .unwrap();

        assert_eq!(resp.blocks.len(), 1);
        let ContentBlock::ToolUse(call) = &resp.blocks[0] else {
            panic!("expected ToolUse block");
        };
        assert_eq!(call.name, "echo");
        assert_eq!(call.input["msg"], "hi");
    }

    #[tokio::test]
    async fn incomplete_tool_emits_failure_chunk() {
        // Stream ends while a tool call is in progress.
        let events = vec![StreamEvent::ToolUseStart {
            id: "t1".into(),
            name: "echo".into(),
        }];
        let registry = StreamRegistry::new();
        let session_id = SessionId::new();

        // Should not error, and the incomplete tool block is not included.
        let resp = consume_stream(make_stream(events), &session_id, &registry, "ch", None)
            .await
            .unwrap();

        assert!(resp.blocks.is_empty());
    }
}
