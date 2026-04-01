//! Onboarding wizard session: LLM-driven configuration loop.
//!
//! [`OnboardSession`] manages a streaming tool-use conversation with an LLM,
//! calling [`OnboardIo`] for terminal I/O and
//! [`orka_core::traits::SecretManager`] for secret storage.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt as _;
use orka_core::{Error, Result, SecretValue, traits::SecretManager};
use orka_llm::{
    ChatContent, ChatMessage, CompletionOptions, ContentBlock, ContentBlockInput, LlmClient, Role,
    StopReason, StreamEvent, ToolCall,
};
use tracing::debug;

use crate::{config_builder::ConfigBuilder, system_prompt::wizard_system_prompt, tools::all_tools};

// ── I/O trait ────────────────────────────────────────────────────────────────

/// Terminal I/O interface for the onboarding wizard.
///
/// Implementors handle rendering to the terminal and collecting user input.
/// A test double can capture events without any actual terminal.
#[async_trait]
pub trait OnboardIo: Send {
    /// Called for each streaming text delta from the LLM.
    async fn on_text_delta(&mut self, delta: &str);

    /// Called at the end of a complete LLM text response.
    async fn on_text_done(&mut self);

    /// Ask the user to enter a masked secret value.
    async fn ask_secret(&mut self, prompt: &str) -> Result<String>;

    /// Ask the user a free-text or selection question.
    ///
    /// `options` is `Some` for selection questions; `None` for free text.
    /// `multi_select` allows picking multiple options.
    async fn ask_input(
        &mut self,
        question: &str,
        options: Option<&[String]>,
        multi_select: bool,
    ) -> Result<Vec<String>>;

    /// Called when the config has been updated (optional preview).
    async fn on_config_updated(&mut self, toml_preview: &str);
}

// ── Session
// ───────────────────────────────────────────────────────────────────

/// Provider metadata captured during Phase 1 bootstrap.
#[derive(Debug, Clone)]
pub struct BootstrapProvider {
    /// Provider identifier (`"anthropic"`, `"openai"`, `"ollama"`, `"custom"`).
    pub provider: String,
    /// Model identifier used for the wizard.
    pub model: String,
    /// Secret store path where the API key was saved (if any).
    pub api_key_secret: Option<String>,
    /// Environment variable name for the API key (if any).
    pub api_key_env: Option<String>,
    /// Base URL override for Ollama / custom providers.
    pub base_url: Option<String>,
}

/// LLM-driven onboarding session.
///
/// Manages the streaming tool-use conversation loop.  Call [`run`](Self::run)
/// with an [`OnboardIo`] implementation to execute the wizard end-to-end.
pub struct OnboardSession {
    client: Arc<dyn LlmClient>,
    config: ConfigBuilder,
    secrets: Arc<dyn SecretManager>,
    messages: Vec<ChatMessage>,
    provider: BootstrapProvider,
    /// Maximum conversation turns before forcing finalisation.
    max_turns: usize,
}

impl OnboardSession {
    /// Create a new onboarding session.
    pub fn new(
        client: Arc<dyn LlmClient>,
        secrets: Arc<dyn SecretManager>,
        provider: BootstrapProvider,
    ) -> Self {
        Self {
            client,
            config: ConfigBuilder::new(),
            secrets,
            messages: Vec::new(),
            provider,
            max_turns: 40,
        }
    }

    /// Run the onboarding wizard to completion.
    ///
    /// Returns the generated `orka.toml` string.
    pub async fn run(&mut self, io: &mut dyn OnboardIo) -> Result<String> {
        let system = wizard_system_prompt(&self.provider.provider, &self.provider.model);
        let tools = all_tools();

        // Seed the bootstrap provider so the LLM doesn't ask about it again.
        self.seed_bootstrap_provider()?;

        // Kick off with a user greeting.
        self.messages
            .push(ChatMessage::user("I want to configure Orka.".to_string()));

        // Build options once; non-exhaustive so must use Default then assign.
        let mut options = CompletionOptions::default();
        options.model = Some(self.provider.model.clone());
        options.max_tokens = Some(4096);

        let mut turn = 0usize;
        loop {
            if turn >= self.max_turns {
                self.messages.push(ChatMessage::user(
                    "Please finalize the configuration now with what we have.".to_string(),
                ));
            }
            turn += 1;

            debug!(turn, "starting wizard turn");

            let stream = self
                .client
                .complete_stream_with_tools(&self.messages, &system, &tools, &options)
                .await?;

            let result = consume_stream(stream, io).await?;

            // Append assistant message with all content blocks.
            let assistant_blocks: Vec<ContentBlockInput> = result
                .content_blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text(t) => ContentBlockInput::Text { text: t.clone() },
                    ContentBlock::ToolUse(tc) => ContentBlockInput::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                    },
                    ContentBlock::Thinking(t) => ContentBlockInput::Thinking {
                        thinking: t.clone(),
                    },
                    // Non-exhaustive: ignore future variants.
                    _ => ContentBlockInput::Text {
                        text: String::new(),
                    },
                })
                .collect();

            self.messages.push(ChatMessage::new(
                Role::Assistant,
                ChatContent::Blocks(assistant_blocks),
            ));

            // No tool calls → wait for user input.
            if result.tool_calls.is_empty() {
                if result.stop_reason == Some(StopReason::EndTurn) {
                    let answers = io
                        .ask_input("Your reply (or press Enter to finish):", None, false)
                        .await?;
                    let user_text = answers.into_iter().next().unwrap_or_default();
                    if user_text.trim().is_empty() {
                        break;
                    }
                    self.messages.push(ChatMessage::user(user_text));
                    continue;
                }
                break;
            }

            // Process tool calls.
            let mut finalized = false;
            let mut tool_results: Vec<ContentBlockInput> = Vec::new();

            for call in &result.tool_calls {
                let (content, is_error) = match self.handle_tool_call(call, io).await {
                    Ok(msg) => (msg, false),
                    Err(e) => (format!("Error: {e}"), true),
                };

                if call.name == "finalize" && !is_error {
                    finalized = true;
                }

                tool_results.push(ContentBlockInput::ToolResult {
                    tool_use_id: call.id.clone(),
                    content,
                    is_error,
                });
            }

            self.messages.push(ChatMessage::new(
                Role::User,
                ChatContent::Blocks(tool_results),
            ));

            if finalized {
                break;
            }
        }

        Ok(self.config.to_toml())
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Pre-populate the bootstrap provider into the config.
    pub(crate) fn seed_bootstrap_provider(&mut self) -> Result<()> {
        let mut entry = serde_json::json!({
            "name": self.provider.provider,
            "provider": self.provider.provider,
            "model": self.provider.model,
        });

        if let Some(path) = &self.provider.api_key_secret {
            entry["api_key_secret"] = serde_json::Value::String(path.clone());
        } else if let Some(env) = &self.provider.api_key_env {
            entry["api_key_env"] = serde_json::Value::String(env.clone());
        }

        if let Some(url) = &self.provider.base_url {
            entry["base_url"] = serde_json::Value::String(url.clone());
        }

        self.config.append_array_entry("llm.providers", &entry)
    }

    /// Dispatch a single tool call and return a result string.
    async fn handle_tool_call(
        &mut self,
        call: &ToolCall,
        io: &mut dyn OnboardIo,
    ) -> Result<String> {
        debug!(tool = %call.name, "handling tool call");

        match call.name.as_str() {
            "set_config" => {
                let section = call
                    .input
                    .get("section")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::Config("set_config: missing `section`".to_string()))?;
                let values = call
                    .input
                    .get("values")
                    .ok_or_else(|| Error::Config("set_config: missing `values`".to_string()))?;

                if ConfigBuilder::is_array_of_tables(section) {
                    return Err(Error::Config(format!(
                        "'{section}' is an array-of-tables — use `append_config` instead"
                    )));
                }

                self.config.set_section(section, values)?;
                io.on_config_updated(&self.config.to_toml()).await;
                Ok(format!("Config section [{section}] updated."))
            }

            "append_config" => {
                let section = call
                    .input
                    .get("section")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::Config("append_config: missing `section`".to_string()))?;
                let entry = call
                    .input
                    .get("entry")
                    .ok_or_else(|| Error::Config("append_config: missing `entry`".to_string()))?;

                self.config.append_array_entry(section, entry)?;
                io.on_config_updated(&self.config.to_toml()).await;
                Ok(format!("Entry appended to [[{section}]]."))
            }

            "store_secret" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::Config("store_secret: missing `path`".to_string()))?;
                let prompt = call
                    .input
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Enter secret value");

                let value = io.ask_secret(prompt).await?;
                if value.is_empty() {
                    return Err(Error::Config("secret value cannot be empty".to_string()));
                }

                self.secrets
                    .set_secret(path, &SecretValue::new(value.into_bytes()))
                    .await?;

                Ok(format!(
                    "Secret stored at path \"{path}\". \
                     Reference it in config as `api_key_secret = \"{path}\"` (or the \
                     appropriate field for this secret type)."
                ))
            }

            "ask_user" => {
                let question = call
                    .input
                    .get("question")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::Config("ask_user: missing `question`".to_string()))?;
                let options: Option<Vec<String>> = call
                    .input
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    });
                let multi_select = call
                    .input
                    .get("multi_select")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);

                let answers = io
                    .ask_input(question, options.as_deref(), multi_select)
                    .await?;

                Ok(answers.join(", "))
            }

            "validate_config" => match self.config.validate() {
                Ok(_) => Ok("Config is valid.".to_string()),
                Err(e) => Ok(format!("Validation issues: {e}")),
            },

            "finalize" => {
                let summary = call
                    .input
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Configuration complete.");
                Ok(format!("Wizard finalised. {summary}"))
            }

            unknown => Err(Error::Config(format!("unknown tool: {unknown}"))),
        }
    }
}

// ── Stream consumer
// ───────────────────────────────────────────────────────────

struct TurnResult {
    content_blocks: Vec<ContentBlock>,
    tool_calls: Vec<ToolCall>,
    stop_reason: Option<StopReason>,
}

/// Consume an LLM tool stream, forwarding text deltas to `io` and
/// accumulating the full response.
async fn consume_stream(
    mut stream: orka_llm::LlmToolStream,
    io: &mut dyn OnboardIo,
) -> Result<TurnResult> {
    let mut text_buf = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut active_id: Option<String> = None;
    let mut active_name: Option<String> = None;
    let mut stop_reason: Option<StopReason> = None;

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta(delta) => {
                io.on_text_delta(&delta).await;
                text_buf.push_str(&delta);
            }
            StreamEvent::ToolUseStart { id, name } => {
                active_id = Some(id);
                active_name = Some(name);
            }
            // Input is delivered whole in ToolUseEnd; deltas ignored here.
            StreamEvent::ToolUseEnd { id, input } => {
                let resolved_id = active_id.take().unwrap_or(id);
                let name = active_name.take().unwrap_or_default();
                tool_calls.push(ToolCall::new(resolved_id, name, input));
            }
            StreamEvent::Stop(reason) => {
                stop_reason = Some(reason);
            }
            // Non-exhaustive enum: ignore new variants added in future versions.
            _ => {}
        }
    }

    if !text_buf.is_empty() {
        io.on_text_done().await;
    }

    let mut content_blocks: Vec<ContentBlock> = Vec::new();
    if !text_buf.is_empty() {
        content_blocks.push(ContentBlock::Text(text_buf));
    }
    for tc in &tool_calls {
        content_blocks.push(ContentBlock::ToolUse(tc.clone()));
    }

    Ok(TurnResult {
        content_blocks,
        tool_calls,
        stop_reason,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Arc;

    use orka_core::testing::InMemorySecretManager;

    use super::*;

    fn make_provider() -> BootstrapProvider {
        BootstrapProvider {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            api_key_secret: Some("llm/anthropic".to_string()),
            api_key_env: None,
            base_url: None,
        }
    }

    /// Minimal `LlmClient` stub that always returns an empty stream.
    struct NoOpClient;

    #[async_trait::async_trait]
    impl LlmClient for NoOpClient {
        async fn complete(
            &self,
            _messages: Vec<ChatMessage>,
            _system: &str,
        ) -> orka_core::Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn seed_bootstrap_provider_sets_provider_entry() {
        let secrets = Arc::new(InMemorySecretManager::new());
        let client = Arc::new(NoOpClient);
        let provider = make_provider();
        let mut session = OnboardSession::new(client, secrets, provider);
        session.seed_bootstrap_provider().unwrap();
        let toml = session.config.to_toml();
        assert!(toml.contains("[[llm.providers]]"));
        assert!(toml.contains("provider = \"anthropic\""));
        assert!(toml.contains("api_key_secret = \"llm/anthropic\""));
    }

    #[test]
    fn seed_bootstrap_provider_with_env_var() {
        let secrets = Arc::new(InMemorySecretManager::new());
        let client = Arc::new(NoOpClient);
        let provider = BootstrapProvider {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key_secret: None,
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            base_url: None,
        };
        let mut session = OnboardSession::new(client, secrets, provider);
        session.seed_bootstrap_provider().unwrap();
        let toml = session.config.to_toml();
        assert!(toml.contains("api_key_env = \"OPENAI_API_KEY\""));
    }

    #[test]
    fn seed_bootstrap_provider_with_base_url() {
        let secrets = Arc::new(InMemorySecretManager::new());
        let client = Arc::new(NoOpClient);
        let provider = BootstrapProvider {
            provider: "ollama".to_string(),
            model: "llama3".to_string(),
            api_key_secret: None,
            api_key_env: None,
            base_url: Some("http://localhost:11434/v1".to_string()),
        };
        let mut session = OnboardSession::new(client, secrets, provider);
        session.seed_bootstrap_provider().unwrap();
        let toml = session.config.to_toml();
        assert!(toml.contains("base_url"));
        assert!(toml.contains("ollama"));
    }
}
