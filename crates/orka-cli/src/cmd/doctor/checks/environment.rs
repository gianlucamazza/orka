use std::process::Stdio;

use async_trait::async_trait;

use crate::cmd::doctor::{
    CheckContext, DoctorCheck,
    types::{Category, CheckId, CheckMeta, CheckOutcome, Severity},
};

pub struct EnvRustToolchain;
pub struct EnvDockerAvailable;
pub struct EnvOsCapabilities;
pub struct EnvMcpBinaries;
pub struct EnvPluginDir;
pub struct EnvAdapterTokens;

const MSRV: &str = "1.85";

#[async_trait]
impl DoctorCheck for EnvRustToolchain {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("ENV-001"),
            category: Category::Environment,
            severity: Severity::Info,
            name: "Rust toolchain",
            description: "Reports the active Rust version (MSRV 1.85).",
        }
    }

    async fn run(&self, _ctx: &CheckContext) -> CheckOutcome {
        match tokio::process::Command::new("rustc")
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
                // Parse version number like "rustc 1.85.0 (4d91de4e0 2025-03-01)"
                if let Some(version_str) = ver.split_whitespace().nth(1) {
                    if version_ok(version_str, MSRV) {
                        CheckOutcome::pass(ver)
                    } else {
                        CheckOutcome::fail(format!("{ver} — below MSRV {MSRV}")).with_hint(format!(
                            "Upgrade to Rust {MSRV} or later with `rustup update stable`."
                        ))
                    }
                } else {
                    CheckOutcome::pass(ver)
                }
            }
            _ => CheckOutcome::skip("rustc not found in PATH"),
        }
    }
}

/// Returns true if `version_str` >= `min`.
fn version_ok(version_str: &str, min: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut parts = s.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(version_str) >= parse(min)
}

#[async_trait]
impl DoctorCheck for EnvDockerAvailable {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("ENV-002"),
            category: Category::Environment,
            severity: Severity::Info,
            name: "Docker available",
            description: "Docker is required for integration tests and docker compose.",
        }
    }

    async fn run(&self, _ctx: &CheckContext) -> CheckOutcome {
        match tokio::process::Command::new("docker")
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
                CheckOutcome::pass(ver)
            }
            _ => CheckOutcome::skip(
                "docker not found in PATH (optional — needed for integration tests)",
            ),
        }
    }
}

#[async_trait]
impl DoctorCheck for EnvOsCapabilities {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("ENV-003"),
            category: Category::Environment,
            severity: Severity::Info,
            name: "OS capabilities",
            description: "Probes available OS tools: package manager, systemctl, journalctl, claude, codex.",
        }
    }

    async fn run(&self, _ctx: &CheckContext) -> CheckOutcome {
        let probes: &[(&str, &[&str])] = &[
            ("pacman", &["--version"]),
            ("apt", &["--version"]),
            ("dnf", &["--version"]),
            ("systemctl", &["--version"]),
            ("journalctl", &["--version"]),
            ("claude", &["--version"]),
            ("codex", &["--version"]),
        ];

        let mut found = Vec::new();
        let mut not_found = Vec::new();

        for (cmd, args) in probes {
            let available = tokio::process::Command::new(cmd)
                .args(*args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);

            if available {
                found.push(*cmd);
            } else {
                not_found.push(*cmd);
            }
        }

        let msg = if found.is_empty() {
            "no OS tools found".to_string()
        } else {
            format!("found: {}", found.join(", "))
        };

        CheckOutcome::pass(msg).with_detail(format!(
            "found: [{}] | not found: [{}]",
            found.join(", "),
            not_found.join(", ")
        ))
    }
}

#[async_trait]
impl DoctorCheck for EnvMcpBinaries {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("ENV-004"),
            category: Category::Environment,
            severity: Severity::Warning,
            name: "MCP server binaries",
            description: "Command binaries for stdio MCP servers must exist in PATH.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        let stdio_servers: Vec<_> = config
            .mcp
            .servers
            .iter()
            .filter(|s| s.transport == "stdio")
            .collect();

        if stdio_servers.is_empty() {
            return CheckOutcome::skip("no stdio MCP servers configured");
        }

        let mut missing = Vec::new();
        let mut found = Vec::new();

        for server in &stdio_servers {
            let Some(cmd) = &server.command else {
                missing.push(format!("{} (no command)", server.name));
                continue;
            };

            let in_path = which_binary(cmd).await;
            if in_path {
                found.push(server.name.clone());
            } else {
                missing.push(format!("{} ({})", server.name, cmd));
            }
        }

        if missing.is_empty() {
            CheckOutcome::pass(format!("{} MCP server(s) OK", found.len()))
                .with_detail(found.join(", "))
        } else {
            CheckOutcome::fail(format!(
                "{} MCP server binary(ies) not found",
                missing.len()
            ))
            .with_detail(format!(
                "missing: {} | found: {}",
                missing.join(", "),
                found.join(", ")
            ))
            .with_hint("Install the missing binaries or remove the MCP server entries from config.")
        }
    }
}

async fn which_binary(cmd: &str) -> bool {
    // Use `which` to check if binary is in PATH
    tokio::process::Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        // Also try direct path if the command contains /
        .unwrap_or_else(|_| std::path::Path::new(cmd).exists())
}

#[async_trait]
impl DoctorCheck for EnvPluginDir {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("ENV-005"),
            category: Category::Environment,
            severity: Severity::Warning,
            name: "Plugin directory",
            description: "The configured WASM plugin directory must exist and contain .wasm files.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        let Some(dir) = &config.plugins.dir else {
            return CheckOutcome::skip("plugins.dir not configured");
        };

        let path = std::path::Path::new(dir);
        if !path.exists() {
            return CheckOutcome::fail(format!("plugin dir not found: {dir}"))
                .with_hint("Create the directory or update plugins.dir in orka.toml.");
        }

        // Count .wasm files
        match std::fs::read_dir(path) {
            Ok(entries) => {
                let wasm_count = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext == "wasm")
                            .unwrap_or(false)
                    })
                    .count();

                if wasm_count == 0 {
                    CheckOutcome::pass(format!("{dir} exists (0 .wasm files)"))
                        .with_detail("No WASM plugins loaded — directory is empty.")
                } else {
                    CheckOutcome::pass(format!("{dir} ({wasm_count} .wasm file(s))"))
                }
            }
            Err(e) => CheckOutcome::fail(format!("cannot read plugin dir: {e}")),
        }
    }
}

#[async_trait]
impl DoctorCheck for EnvAdapterTokens {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("ENV-006"),
            category: Category::Environment,
            severity: Severity::Error,
            name: "Adapter tokens configured",
            description: "Enabled adapters must have bot_token_secret or equivalent configured.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        let mut configured = Vec::new();
        let mut missing = Vec::new();

        if let Some(tg) = &config.adapters.telegram {
            if tg
                .bot_token_secret
                .as_deref()
                .is_some_and(|s| !s.is_empty())
            {
                configured.push("telegram");
            } else {
                missing.push("telegram (bot_token_secret not set)");
            }
        }

        if let Some(dc) = &config.adapters.discord {
            if dc
                .bot_token_secret
                .as_deref()
                .is_some_and(|s| !s.is_empty())
            {
                configured.push("discord");
            } else {
                missing.push("discord (bot_token_secret not set)");
            }
        }

        if let Some(sl) = &config.adapters.slack {
            // Slack uses bot_token_secret too (check field existence)
            let has_token = sl
                .bot_token_secret
                .as_deref()
                .is_some_and(|s| !s.is_empty());
            if has_token {
                configured.push("slack");
            } else {
                missing.push("slack (bot_token_secret not set)");
            }
        }

        if let Some(wa) = &config.adapters.whatsapp {
            let has_token = wa
                .access_token_secret
                .as_deref()
                .is_some_and(|s| !s.is_empty());
            if has_token {
                configured.push("whatsapp");
            } else {
                missing.push("whatsapp (access_token_secret not set)");
            }
        }

        if config.adapters.telegram.is_none()
            && config.adapters.discord.is_none()
            && config.adapters.slack.is_none()
            && config.adapters.whatsapp.is_none()
            && config.adapters.custom.is_none()
        {
            return CheckOutcome::skip("no adapters configured");
        }

        if missing.is_empty() {
            CheckOutcome::pass(format!(
                "{} adapter(s) configured: {}",
                configured.len(),
                configured.join(", ")
            ))
        } else {
            CheckOutcome::fail(format!(
                "{} adapter(s) missing token: {}",
                missing.len(),
                missing.join(", ")
            ))
            .with_hint(
                "Run `orka secret set <path>` to store the bot token, then set \
                 bot_token_secret = \"<path>\" in the adapter config.",
            )
        }
    }
}
