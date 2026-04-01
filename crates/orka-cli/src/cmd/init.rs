//! `orka init` — LLM-driven onboarding wizard.
//!
//! # Two-phase bootstrap
//!
//! **Phase 1** collects the minimum required to obtain a working LLM client
//! (provider choice + API key) via traditional `dialoguer` prompts, then
//! validates the key with a test call.
//!
//! **Phase 2** delegates entirely to [`OnboardSession`], which drives a
//! streaming tool-use conversation to configure the rest of `orka.toml`.

use std::{io::Write as _, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use colored::Colorize as _;
use orka_core::SecretStr;
use orka_llm::{
    ANTHROPIC_API_VERSION, AnthropicClient, CompletionOptions, LlmClient, OllamaClient,
    OpenAiClient,
};
use crate::onboard::{BootstrapProvider, OnboardIo, OnboardSession};
use orka_secrets::create_file_secret_manager;

use crate::client::Result;

// ── Argument definition (parsed in main.rs) ──────────────────────────────────

/// Arguments for `orka init`.
#[derive(Debug)]
pub struct InitArgs {
    /// LLM provider (skip interactive prompt if set).
    pub provider: Option<String>,
    /// API key (skip interactive prompt if set).
    pub api_key: Option<String>,
    /// Model override.
    pub model: Option<String>,
    /// Base URL for Ollama / custom OpenAI-compatible providers.
    pub base_url: Option<String>,
    /// Output path for the generated config.
    pub output: String,
    /// Generate minimal config without LLM conversation.
    pub minimal: bool,
    /// Extend an existing config instead of overwriting.
    pub extend: bool,
}

// ── Entry point
// ───────────────────────────────────────────────────────────────

pub async fn run(args: InitArgs) -> Result<()> {
    print_banner();

    let output_path = PathBuf::from(&args.output);

    // Guard against accidental overwrites.
    if output_path.exists() && !args.extend {
        let ok = dialoguer::Confirm::new()
            .with_prompt(format!(
                "{} already exists. Overwrite?",
                args.output.yellow()
            ))
            .default(false)
            .interact()?;
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    // ── Phase 1: Bootstrap ────────────────────────────────────────────────────

    let (client, provider_info) = phase1_bootstrap(&args)?;

    if args.minimal {
        return write_minimal_config(&output_path, &provider_info);
    }

    // ── Phase 2: LLM-driven wizard ────────────────────────────────────────────

    let secrets_path = orka_secrets::default_secrets_file_path();
    let secrets = create_file_secret_manager(&secrets_path)?;

    // Store bootstrap API key in the secret store (if provided).
    if let Some(key_path) = &provider_info.api_key_secret
        && let Some(key_val) = args.api_key.as_deref()
    {
        secrets
            .set_secret(
                key_path,
                &orka_core::SecretValue::new(key_val.as_bytes().to_vec()),
            )
            .await?;
    }

    let mut session = OnboardSession::new(client, secrets, provider_info);
    let mut io = TerminalIo::new();

    println!(
        "\n{}\n",
        "Starting the Orka configuration wizard...".cyan().bold()
    );

    let toml = session.run(&mut io).await?;

    // Write the generated config.
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, &toml)?;

    println!(
        "\n{} {}",
        "Config written to".green().bold(),
        args.output.yellow()
    );

    // Post-wizard validation via `orka config check`.
    println!("\n{}", "Validating generated config...".cyan());
    match orka_config::OrkaConfig::load(Some(&output_path)) {
        Ok(mut cfg) => match cfg.validate() {
            Ok(()) => println!("{}", "Config validation: OK".green()),
            Err(e) => println!("{} {}", "Validation warning:".yellow(), e),
        },
        Err(e) => println!("{} {}", "Could not load config:".yellow(), e),
    }

    println!(
        "\n{}\n",
        "Run `orka` to start the server, or `orka doctor` to run a full diagnostic."
            .white()
            .bold()
    );

    Ok(())
}

// ── Phase 1
// ───────────────────────────────────────────────────────────────────

/// Supported LLM providers for the wizard bootstrap.
const PROVIDERS: &[&str] = &[
    "Anthropic (Claude)",
    "Moonshot (Kimi)",
    "OpenAI (GPT)",
    "Google (Gemini)",
    "Ollama (local)",
    "Custom (OpenAI-compatible)",
];

/// Default models for each provider.
fn default_model(provider_key: &str) -> &'static str {
    match provider_key {
        "anthropic" => "claude-sonnet-4-6",
        "moonshot" => "kimi-k2-thinking-turbo",
        "google" => "gemini-2.0-flash",
        "ollama" => "llama3",
        _ => "gpt-4o",
    }
}

/// Default base URLs for providers that need one.
fn default_base_url(provider_key: &str) -> Option<String> {
    match provider_key {
        "moonshot" => Some("https://api.moonshot.ai/v1".to_string()),
        "ollama" => Some("http://localhost:11434/v1".to_string()),
        _ => None,
    }
}

/// Default env var name for the API key.
fn default_env_var(provider_key: &str) -> Option<&'static str> {
    match provider_key {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "moonshot" => Some("MOONSHOT_API_KEY"),
        "openai" => Some("OPENAI_API_KEY"),
        "google" => Some("GEMINI_API_KEY"),
        _ => None,
    }
}

/// Canonical provider key from display name index.
fn provider_key(idx: usize) -> &'static str {
    match idx {
        0 => "anthropic",
        1 => "moonshot",
        2 => "openai",
        3 => "google",
        4 => "ollama",
        _ => "custom",
    }
}

/// Phase 1: collect provider + API key, build and test the LLM client.
fn phase1_bootstrap(args: &InitArgs) -> Result<(Arc<dyn LlmClient>, BootstrapProvider)> {
    // --- Provider selection ---
    let (p_key, api_key_input, base_url_input, model_input) =
        if let (Some(p), Some(k)) = (args.provider.as_deref(), args.api_key.as_deref()) {
            (
                p.to_lowercase(),
                Some(k.to_string()),
                args.base_url.clone(),
                args.model.clone(),
            )
        } else {
            interactive_phase1(args)?
        };

    let model = model_input.unwrap_or_else(|| default_model(&p_key).to_string());
    let base_url = base_url_input.or_else(|| default_base_url(&p_key));

    // --- Resolve API key source ---
    // Check env var first, then use the interactively entered value.
    let (api_key_value, api_key_secret, api_key_env) =
        resolve_key_source(&p_key, api_key_input.as_deref(), &model);

    // --- Build client ---
    let client = build_client(
        &p_key,
        &model,
        api_key_value.as_deref(),
        base_url.as_deref(),
    )?;

    // --- Test the connection ---
    println!("\n{}", "Testing LLM connection...".cyan());
    let test_client = Arc::clone(&client);
    let rt = tokio::runtime::Handle::current();
    let test_result = rt.block_on(async move {
        let mut opts = CompletionOptions::default();
        opts.max_tokens = Some(16);
        test_client
            .complete_with_options(
                vec![orka_llm::ChatMessage::user("Reply with: OK".to_string())],
                "You are a test. Reply with exactly 'OK' and nothing else.",
                &opts,
            )
            .await
    });

    match test_result {
        Ok(_) => println!("{}", "Connection successful.".green()),
        Err(e) => {
            eprintln!(
                "{} {}\n{}",
                "Connection failed:".red().bold(),
                e,
                "Please check your API key and try again.".yellow()
            );
            return Err(format!("LLM test call failed: {e}").into());
        }
    }

    let provider_info = BootstrapProvider {
        provider: p_key,
        model,
        api_key_secret,
        api_key_env,
        base_url,
    };

    Ok((client, provider_info))
}

/// Collect provider, API key, base URL, and model interactively.
#[allow(clippy::type_complexity)]
fn interactive_phase1(
    args: &InitArgs,
) -> Result<(String, Option<String>, Option<String>, Option<String>)> {
    let p_idx = dialoguer::Select::new()
        .with_prompt("LLM provider")
        .items(PROVIDERS)
        .default(0)
        .interact()?;

    let p_key = provider_key(p_idx).to_string();
    let needs_api_key = p_key != "ollama";
    let needs_base_url = matches!(p_key.as_str(), "moonshot" | "ollama" | "custom");

    let api_key_input = if needs_api_key {
        // Check if env var is already set.
        let env = default_env_var(&p_key);
        if let Some(env_name) = env {
            if std::env::var(env_name).is_ok() {
                println!(
                    "{} {} {}",
                    "Using API key from".cyan(),
                    env_name.yellow(),
                    "environment variable.".cyan()
                );
                None // Will be resolved from env at runtime
            } else {
                let key = dialoguer::Password::new()
                    .with_prompt(format!("API key (or set {env_name} env var)"))
                    .interact()?;
                if key.is_empty() {
                    return Err("API key must not be empty".into());
                }
                Some(key)
            }
        } else {
            let key = dialoguer::Password::new()
                .with_prompt("API key")
                .interact()?;
            if key.is_empty() {
                return Err("API key must not be empty".into());
            }
            Some(key)
        }
    } else {
        None
    };

    let base_url_input = if needs_base_url {
        let default =
            default_base_url(&p_key).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let url: String = dialoguer::Input::new()
            .with_prompt("Base URL")
            .default(default)
            .interact_text()?;
        Some(url)
    } else {
        args.base_url.clone()
    };

    let default_m = args
        .model
        .clone()
        .unwrap_or_else(|| default_model(&p_key).to_string());
    let model_input: String = dialoguer::Input::new()
        .with_prompt("Model")
        .default(default_m)
        .interact_text()?;

    Ok((p_key, api_key_input, base_url_input, Some(model_input)))
}

/// Determine how the API key is referenced in config.
///
/// Returns `(api_key_value_for_test, secret_path_option, env_var_option)`.
fn resolve_key_source(
    provider: &str,
    api_key_input: Option<&str>,
    _model: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    // If the user provided a key interactively → store in secret store.
    if let Some(key) = api_key_input {
        let secret_path = format!("llm/{provider}");
        return (Some(key.to_string()), Some(secret_path), None);
    }
    // Key is from env var → reference it in config as api_key_env.
    if let Some(env) = default_env_var(provider)
        && let Ok(val) = std::env::var(env)
    {
        return (Some(val), None, Some(env.to_string()));
    }
    // Ollama: no key needed.
    (None, None, None)
}

/// Construct an `Arc<dyn LlmClient>` for the given provider.
fn build_client(
    provider: &str,
    model: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Arc<dyn LlmClient>> {
    let client: Arc<dyn LlmClient> = match provider {
        "anthropic" => {
            let key = SecretStr::new(api_key.ok_or("Anthropic API key is required")?);
            Arc::new(AnthropicClient::with_options(
                key,
                model.to_string(),
                120,
                4096,
                2,
                ANTHROPIC_API_VERSION.to_string(),
                base_url.map(str::to_string),
            ))
        }
        "openai" | "moonshot" | "google" | "custom" => {
            let key = SecretStr::new(api_key.unwrap_or(""));
            let url = base_url.map_or_else(
                || {
                    default_base_url(provider)
                        .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
                },
                str::to_string,
            );
            Arc::new(OpenAiClient::with_options(
                key,
                model.to_string(),
                120,
                4096,
                2,
                url,
            ))
        }
        "ollama" => {
            let url =
                base_url.map_or_else(|| "http://localhost:11434/v1".to_string(), str::to_string);
            Arc::new(OllamaClient::with_options(
                model.to_string(),
                120,
                4096,
                1,
                url,
            ))
        }
        other => return Err(format!("unsupported provider: {other}").into()),
    };
    Ok(client)
}

// ── Minimal mode
// ──────────────────────────────────────────────────────────────

/// Write a minimal working `orka.toml` without LLM conversation.
fn write_minimal_config(output: &PathBuf, provider: &BootstrapProvider) -> Result<()> {
    use crate::onboard::ConfigBuilder;

    let mut builder = ConfigBuilder::new();

    let mut entry = serde_json::json!({
        "name": provider.provider,
        "provider": provider.provider,
        "model": provider.model,
    });
    if let Some(p) = &provider.api_key_secret {
        entry["api_key_secret"] = serde_json::Value::String(p.clone());
    } else if let Some(e) = &provider.api_key_env {
        entry["api_key_env"] = serde_json::Value::String(e.clone());
    }

    builder.append_array_entry("llm.providers", &entry)?;
    builder.append_array_entry(
        "agents",
        &serde_json::json!({
            "id": "assistant",
            "kind": "agent",
            "name": "Assistant",
            "system_prompt": "You are a helpful assistant."
        }),
    )?;

    let toml = builder.to_toml();
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &toml)?;

    println!(
        "{} {}",
        "Minimal config written to".green().bold(),
        output.display().to_string().yellow()
    );
    Ok(())
}

// ── Terminal I/O
// ──────────────────────────────────────────────────────────────

struct TerminalIo {
    show_config_preview: bool,
}

impl TerminalIo {
    fn new() -> Self {
        Self {
            show_config_preview: false,
        }
    }
}

#[async_trait]
impl OnboardIo for TerminalIo {
    async fn on_text_delta(&mut self, delta: &str) {
        // Print streaming deltas immediately without newline.
        print!("{delta}");
        let _ = std::io::stdout().flush();
    }

    async fn on_text_done(&mut self) {
        // Ensure the text ends on its own line.
        println!();
    }

    async fn ask_secret(&mut self, prompt: &str) -> orka_core::Result<String> {
        println!();
        let value = dialoguer::Password::new()
            .with_prompt(prompt)
            .interact()
            .map_err(|e| orka_core::Error::Config(format!("prompt error: {e}")))?;
        Ok(value)
    }

    async fn ask_input(
        &mut self,
        question: &str,
        options: Option<&[String]>,
        multi_select: bool,
    ) -> orka_core::Result<Vec<String>> {
        println!();
        match options {
            Some(opts) if !opts.is_empty() => {
                if multi_select {
                    let selections = dialoguer::MultiSelect::new()
                        .with_prompt(question)
                        .items(opts)
                        .interact()
                        .map_err(|e| orka_core::Error::Config(format!("prompt error: {e}")))?;
                    Ok(selections.into_iter().map(|i| opts[i].clone()).collect())
                } else {
                    let idx = dialoguer::Select::new()
                        .with_prompt(question)
                        .items(opts)
                        .default(0)
                        .interact()
                        .map_err(|e| orka_core::Error::Config(format!("prompt error: {e}")))?;
                    Ok(vec![opts[idx].clone()])
                }
            }
            _ => {
                // Free-text input; empty means "done / skip".
                let text: String = dialoguer::Input::new()
                    .with_prompt(question)
                    .allow_empty(true)
                    .interact_text()
                    .map_err(|e| orka_core::Error::Config(format!("prompt error: {e}")))?;
                Ok(vec![text])
            }
        }
    }

    async fn on_config_updated(&mut self, toml_preview: &str) {
        if self.show_config_preview {
            println!(
                "\n{}\n{}\n",
                "── Current config ──".cyan(),
                toml_preview.trim()
            );
        }
    }
}

// ── Welcome banner
// ────────────────────────────────────────────────────────────

fn print_banner() {
    println!(
        "\n{}\n{}\n",
        "┌─────────────────────────────────┐".cyan(),
        format!("│  {} orka init wizard  │", "★".yellow()).cyan()
    );
    println!(
        "{}",
        "This wizard will guide you through configuring Orka.\n\
         At minimum, you need an LLM provider API key.\n"
            .white()
    );
}
