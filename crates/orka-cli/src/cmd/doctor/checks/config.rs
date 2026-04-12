use async_trait::async_trait;
use orka_config::{
    OrkaConfig,
    migrate::{self, CURRENT_CONFIG_VERSION},
};

use crate::cmd::doctor::{
    CheckContext, DoctorCheck,
    types::{Category, CheckId, CheckMeta, CheckOutcome, FixAction, Severity},
};

pub struct CfgFileExists;
pub struct CfgTomlParses;
pub struct CfgVersionCurrent;
pub struct CfgValidation;
pub struct CfgNoDeprecated;
pub struct CfgAgentDefs;
pub struct CfgGraphPresent;

#[async_trait]
impl DoctorCheck for CfgFileExists {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("CFG-001"),
            category: Category::Config,
            severity: Severity::Critical,
            name: "Config file exists",
            description: "The orka.toml configuration file must exist at the resolved path.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        if ctx.config_path.exists() {
            CheckOutcome::pass(format!("{}", ctx.config_path.display()))
        } else {
            let path = ctx.config_path.display().to_string();
            CheckOutcome::fail(format!("not found: {path}"))
                .with_hint("Create orka.toml in the current directory, set ORKA_CONFIG env var, or install the system config at /etc/orka/orka.toml.")
                .with_fix(FixAction {
                    description: format!("Generate a minimal orka.toml at {path}"),
                    apply: Box::new(move || {
                        let minimal = format!(
                            "# Orka configuration (auto-generated)\nconfig_version = {CURRENT_CONFIG_VERSION}\n\n[[agents]]\nid = \"default\"\n"
                        );
                        std::fs::write(&path, minimal)?;
                        Ok(format!("Created {path}"))
                    }),
                })
        }
    }

    fn explain(&self) -> &'static str {
        "Orka resolves the config file in this order: explicit --config flag, \
         ORKA_CONFIG environment variable, ./orka.toml in the current directory, \
         then /etc/orka/orka.toml (system install path). \
         If none is found, all subsequent checks are skipped. \
         Run `orka doctor --fix` to generate a minimal starter config."
    }
}

#[async_trait]
impl DoctorCheck for CfgTomlParses {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("CFG-002"),
            category: Category::Config,
            severity: Severity::Critical,
            name: "TOML syntax valid",
            description: "The configuration file must be valid TOML.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(raw) = &ctx.config_raw else {
            return CheckOutcome::skip("config file not readable");
        };
        match toml::from_str::<toml::Value>(raw) {
            Ok(_) => CheckOutcome::pass("syntax OK"),
            Err(e) => CheckOutcome::fail(format!("TOML parse error: {e}"))
                .with_hint("Fix the syntax error in orka.toml before proceeding."),
        }
    }
}

#[async_trait]
impl DoctorCheck for CfgVersionCurrent {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("CFG-003"),
            category: Category::Config,
            severity: Severity::Warning,
            name: "Config version current",
            description: "The config schema version should match the current version.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(raw) = &ctx.config_raw else {
            return CheckOutcome::skip("config file not readable");
        };
        match migrate::migrate_for_write(raw) {
            Err(e) => CheckOutcome::fail(format!("migration check failed: {e}")),
            Ok((_, None)) => CheckOutcome::pass(format!("v{CURRENT_CONFIG_VERSION} (up to date)")),
            Ok((migrated_toml, Some(res))) => {
                let from = res.from_version;
                let to = res.to_version;
                let config_path = ctx.config_path.clone();
                let message = if from == to {
                    format!("config is v{to}, but canonical rewrite is available.")
                } else {
                    format!("config is v{from}, current is v{to}. Migration available.")
                };
                let description = if from == to {
                    format!("Rewrite config to canonical v{to} schema (backup created)")
                } else {
                    format!("Migrate config v{from} → v{to} (backup created)")
                };
                CheckOutcome::fail(message)
                    .with_hint("Run `orka config migrate` to rewrite the config file.")
                    .with_fix(FixAction {
                        description,
                        apply: Box::new(move || {
                            let backup = config_path.with_extension("toml.bak");
                            std::fs::copy(&config_path, &backup)?;
                            std::fs::write(&config_path, &migrated_toml)?;
                            Ok(format!(
                                "Backup saved to {}, config rewritten to v{to}",
                                backup.display()
                            ))
                        }),
                    })
            }
        }
    }

    fn explain(&self) -> &'static str {
        "Orka uses a sequential migration engine to update config schemas. \
         When the config version is behind, `orka config migrate` applies all pending \
         migrations and writes a backup (.toml.bak). Use `orka config migrate --dry-run` \
         to preview the diff without writing."
    }
}

#[async_trait]
impl DoctorCheck for CfgValidation {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("CFG-004"),
            category: Category::Config,
            severity: Severity::Error,
            name: "Full validation passes",
            description: "OrkaConfig::load() and validate() must succeed.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        if ctx.config_raw.is_none() {
            return CheckOutcome::skip("config file not readable");
        }
        match OrkaConfig::load(Some(&ctx.config_path)) {
            Err(e) => CheckOutcome::fail(format!("load error: {e}"))
                .with_hint("Fix the configuration error reported above."),
            Ok(mut cfg) => match cfg.validate() {
                Ok(()) => CheckOutcome::pass("validation OK"),
                Err(e) => CheckOutcome::fail(format!("validation error: {e}"))
                    .with_hint("See the error message above for the specific field to fix."),
            },
        }
    }
}

#[async_trait]
impl DoctorCheck for CfgNoDeprecated {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("CFG-005"),
            category: Category::Config,
            severity: Severity::Warning,
            name: "No deprecated fields",
            description: "Inline API keys should use api_key_env or api_key_secret instead.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(raw) = &ctx.config_raw else {
            return CheckOutcome::skip("config file not readable");
        };

        // Look for inline api_key values that look like actual keys (not empty, not a
        // path)
        let mut issues = Vec::new();
        if let Ok(doc) = raw.parse::<toml_edit::DocumentMut>() {
            scan_inline_keys(&doc, &mut issues);
        }

        if issues.is_empty() {
            CheckOutcome::pass("no inline API keys detected")
        } else {
            CheckOutcome::fail(format!("{} inline API key(s) found", issues.len()))
                .with_detail(issues.join(", "))
                .with_hint(
                    "Use api_key_env = \"ENV_VAR_NAME\" or api_key_secret = \"path/in/store\" \
                     instead of inline api_key values.",
                )
        }
    }
}

fn scan_inline_keys(doc: &toml_edit::DocumentMut, issues: &mut Vec<String>) {
    // Check web.api_key
    if let Some(web_table) = doc.get("web").and_then(|v| v.as_table())
        && let Some(key) = web_table.get("api_key").and_then(|v| v.as_str())
        && !key.is_empty()
        && !key.contains('/')
    {
        issues.push("web.api_key".to_string());
    }

    // Check llm.providers[].api_key
    let providers_item = doc
        .get("llm")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("providers"));

    if let Some(arr) = providers_item.and_then(|v| v.as_array_of_tables()) {
        for (i, provider) in arr.iter().enumerate() {
            if let Some(key) = provider.get("api_key").and_then(|v| v.as_str())
                && !key.is_empty()
            {
                let name = provider.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                issues.push(format!("llm.providers[{i}] ({name}) api_key"));
            }
        }
    }
}

#[async_trait]
impl DoctorCheck for CfgAgentDefs {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("CFG-006"),
            category: Category::Config,
            severity: Severity::Error,
            name: "Agent definitions valid",
            description: "At least one agent must be defined with a non-empty id.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if config.agents.is_empty() {
            return CheckOutcome::fail("no agents defined")
                .with_hint("Add at least one [[agents]] entry to orka.toml.");
        }

        let empty_ids: Vec<_> = config
            .agents
            .iter()
            .enumerate()
            .filter(|(_, a)| a.id.is_empty())
            .map(|(i, _)| format!("agents[{i}]"))
            .collect();

        if empty_ids.is_empty() {
            CheckOutcome::pass(format!("{} agent(s) defined", config.agents.len()))
        } else {
            CheckOutcome::fail(format!("{} agent(s) have empty id", empty_ids.len()))
                .with_detail(empty_ids.join(", "))
                .with_hint("Each [[agents]] entry must have a non-empty id field.")
        }
    }
}

#[async_trait]
impl DoctorCheck for CfgGraphPresent {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("CFG-007"),
            category: Category::Config,
            severity: Severity::Error,
            name: "Graph config (multi-agent)",
            description: "A [graph] section is required when more than one agent is defined.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if config.agents.len() <= 1 {
            return CheckOutcome::skip(format!(
                "{} agent(s) — graph not required",
                config.agents.len()
            ));
        }

        if config.graph.is_some() {
            CheckOutcome::pass(format!("[graph] present ({} agents)", config.agents.len()))
        } else {
            CheckOutcome::fail(format!(
                "{} agents defined but no [graph] section",
                config.agents.len()
            ))
            .with_hint("Add a [graph] section defining how agents are connected.")
        }
    }
}
