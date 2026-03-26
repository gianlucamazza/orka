use std::collections::HashMap;

use orka_core::{
    DomainEvent, DomainEventKind, Envelope, ErrorCategory, Session, SkillInput,
    traits::GuardrailDecision, truncate_tool_result,
};
use orka_llm::client::{ContentBlockInput, ToolCall};
use tracing::{info, warn};

use super::{WorkspaceHandler, tool_meta};
use crate::stream::{StreamChunk, StreamChunkKind};

impl WorkspaceHandler {
    /// Execute tool calls in parallel, emitting streaming events and domain
    /// events. Returns content blocks with tool results in the original
    /// call order. Built-in workspace tools are intercepted before
    /// dispatching to the skill registry.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn execute_tool_calls(
        &self,
        tool_calls: &[ToolCall],
        envelope: &Envelope,
        session: &Session,
        current_workspace: &str,
    ) -> (Vec<ContentBlockInput>, Vec<Option<ErrorCategory>>) {
        let mut join_set = tokio::task::JoinSet::new();
        // Track which calls are handled as built-in (index → result)
        let mut builtin_results: HashMap<usize, (String, bool)> = HashMap::new();

        for (idx, call) in tool_calls.iter().enumerate() {
            // Check for built-in workspace tools first
            if let Some(result) = self
                .handle_builtin_tool(call, session, current_workspace)
                .await
            {
                info!(tool = %call.name, id = %call.id, "handled built-in workspace tool");
                builtin_results.insert(idx, result);
                continue;
            }

            // Tool input guardrail: check serialized args before executing the skill.
            // Blocked calls are returned to the LLM as an error result (no execution).
            // Modified args replace the original input for the spawned task.
            let mut call_input_override: Option<serde_json::Value> = None;
            if let Some(ref guardrail) = self.guardrail {
                let input_json = call.input.to_string();
                match guardrail.check_input(&input_json, session).await {
                    Ok(GuardrailDecision::Block(reason)) => {
                        warn!(skill = %call.name, %reason, "tool input blocked by guardrail");
                        builtin_results.insert(
                            idx,
                            (format!("Tool input blocked by guardrail: {reason}"), true),
                        );
                        continue;
                    }
                    Ok(GuardrailDecision::Modify(modified)) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&modified) {
                            call_input_override = Some(v);
                        }
                    }
                    _ => {}
                }
            }

            info!(skill = %call.name, id = %call.id, "invoking skill via tool call");

            self.event_sink
                .emit(DomainEvent::new(DomainEventKind::SkillInvoked {
                    skill_name: call.name.clone(),
                    message_id: envelope.id,
                    input_args: match &call.input {
                        serde_json::Value::Object(map) => {
                            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                        }
                        _ => HashMap::new(),
                    },
                    caller_id: None,
                }))
                .await;

            let (category, input_summary) = tool_meta::tool_metadata(&call.name, &call.input);
            self.stream_registry.send(StreamChunk::new(
                envelope.session_id,
                envelope.channel.clone(),
                Some(envelope.id),
                StreamChunkKind::ToolExecStart {
                    name: call.name.clone(),
                    id: call.id.clone(),
                    input_summary,
                    category,
                },
            ));

            let call_id = call.id.clone();
            let call_name = call.name.clone();
            let call_name_for_summary = call.name.clone();
            let call_input = call_input_override.unwrap_or_else(|| call.input.clone());
            let skills = self.skills.clone();
            let event_sink = self.event_sink.clone();
            let message_id = envelope.id;
            let secrets = self.secrets.clone();
            let skill_timeout = std::time::Duration::from_secs(30); // Default timeout
            let max_result_chars = 10000; // Default max chars
            let user_cwd = envelope
                .metadata
                .get("workspace:cwd")
                .and_then(|v| v.as_str())
                .map(String::from);

            join_set.spawn(async move {
                let args: HashMap<String, serde_json::Value> = match call_input {
                    serde_json::Value::Object(map) => map.into_iter().collect(),
                    _ => HashMap::new(),
                };

                let start = std::time::Instant::now();
                let skill_input = SkillInput::new(args).with_context(
                    orka_core::SkillContext::new(secrets, Some(event_sink.clone()))
                        .with_user_cwd(user_cwd),
                );
                let result = match tokio::time::timeout(
                    skill_timeout,
                    skills.invoke(&call_name, skill_input),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => Err(orka_core::Error::Skill(format!(
                        "skill '{call_name}' timed out after {}s",
                        skill_timeout.as_secs()
                    ))),
                };
                let duration_ms = start.elapsed().as_millis() as u64;

                let error_category = match &result {
                    Err(e) => Some(e.category()),
                    Ok(_) => None,
                };
                let (content, is_error) = match &result {
                    Ok(output) => {
                        let raw = output.data.to_string();
                        (truncate_tool_result(&raw, max_result_chars), false)
                    }
                    Err(e) => (format!("Error: {e}"), true),
                };

                let output_preview = match &result {
                    Ok(output) => {
                        let s = output.data.to_string();
                        Some(s.chars().take(1024).collect::<String>())
                    }
                    Err(_) => None,
                };
                let error_message = match &result {
                    Err(e) => Some(e.to_string()),
                    Ok(_) => None,
                };

                event_sink
                    .emit(DomainEvent::new(DomainEventKind::SkillCompleted {
                        skill_name: call_name,
                        message_id,
                        duration_ms,
                        success: !is_error,
                        error_category,
                        output_preview,
                        error_message,
                    }))
                    .await;

                let error_msg = if is_error {
                    Some(content.clone())
                } else {
                    None
                };
                let result_summary =
                    tool_meta::summarize_result(&call_name_for_summary, &content, is_error);
                (
                    call_id,
                    content,
                    is_error,
                    duration_ms,
                    error_msg,
                    result_summary,
                    error_category,
                )
            });
        }

        // Collect results and emit ToolExecEnd chunks
        let mut results_map: HashMap<String, (String, bool, Option<ErrorCategory>)> =
            HashMap::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok((
                call_id,
                content,
                is_error,
                duration_ms,
                error_msg,
                result_summary,
                error_category,
            )) = res
            {
                self.stream_registry.send(StreamChunk::new(
                    envelope.session_id,
                    envelope.channel.clone(),
                    Some(envelope.id),
                    StreamChunkKind::ToolExecEnd {
                        id: call_id.clone(),
                        success: !is_error,
                        duration_ms,
                        error: error_msg,
                        result_summary,
                    },
                ));
                results_map.insert(call_id, (content, is_error, error_category));
            }
        }

        // Build result blocks in original order, merging built-in and skill results
        let mut blocks = Vec::with_capacity(tool_calls.len());
        let mut categories = Vec::with_capacity(tool_calls.len());
        for (idx, call) in tool_calls.iter().enumerate() {
            let (content, is_error, category) =
                if let Some((content, is_error)) = builtin_results.remove(&idx) {
                    (content, is_error, None)
                } else {
                    results_map
                        .remove(&call.id)
                        .unwrap_or_else(|| ("Error: task failed".to_string(), true, None))
                };
            blocks.push(ContentBlockInput::ToolResult {
                tool_use_id: call.id.clone(),
                content,
                is_error,
            });
            categories.push(category);
        }
        (blocks, categories)
    }
}
