//! Config versioning and migration engine.
//!
//! Each config version is an integer. Migrations run sequentially (v0→v1→v2→…)
//! using `toml_edit` to preserve comments and formatting. The server migrates
//! in-memory at boot; only the CLI writes the file.

use std::collections::BTreeSet;

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

/// The config version that this build of Orka expects.
pub const CURRENT_CONFIG_VERSION: u32 = 6;

/// Result of a successful migration.
#[derive(Debug, Clone)]
pub struct MigrationResult {
    /// Version before migration.
    pub from_version: u32,
    /// Version after migration.
    pub to_version: u32,
    /// Deprecation or informational warnings collected during migration.
    pub warnings: Vec<String>,
}

/// Errors that can occur during config migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The config declares a version newer than this build supports.
    #[error(
        "config_version {found} is newer than the maximum supported version {max}; \
         upgrade Orka or downgrade the config"
    )]
    FutureVersion {
        /// Version found in the config file.
        found: u32,
        /// Maximum version this build supports.
        max: u32,
    },

    /// Failed to parse the TOML document.
    #[error("failed to parse config TOML: {0}")]
    Parse(#[from] toml_edit::TomlError),
}

/// Signature for a single migration step.
///
/// Receives a mutable TOML document and a warnings collector.
/// Must update `config_version` in the document to `target_version`.
type MigrateFn = fn(&mut DocumentMut, &mut Vec<String>);

/// Registry of migrations. Each entry is `(target_version, migrate_fn)`.
/// The function migrates from `target_version - 1` to `target_version`.
const MIGRATIONS: &[(u32, MigrateFn)] = &[
    (1, migrate_v0_to_v1),
    (2, migrate_v1_to_v2),
    (3, migrate_v2_to_v3),
    (4, migrate_v3_to_v4),
    (5, migrate_v4_to_v5),
    (6, migrate_v5_to_v6),
];

/// Read `config_version` from a parsed TOML document. Returns 0 if absent.
fn read_version(doc: &DocumentMut) -> u32 {
    doc.get("config_version")
        .and_then(toml_edit::Item::as_integer)
        .map_or(0, |v| v as u32)
}

/// Migrate raw TOML config text if needed.
///
/// Returns the (possibly transformed) TOML string and an optional
/// [`MigrationResult`] if any migration was applied.
pub fn migrate_if_needed(raw: &str) -> Result<(String, Option<MigrationResult>), MigrationError> {
    let mut doc: DocumentMut = raw.parse()?;
    let from_version = read_version(&doc);

    // Reject configs from the future.
    if from_version > CURRENT_CONFIG_VERSION {
        return Err(MigrationError::FutureVersion {
            found: from_version,
            max: CURRENT_CONFIG_VERSION,
        });
    }

    let mut warnings = Vec::new();

    if from_version < CURRENT_CONFIG_VERSION {
        // Apply each step sequentially.
        for &(target, migrate_fn) in MIGRATIONS {
            if target <= from_version {
                continue;
            }
            if target > CURRENT_CONFIG_VERSION {
                break;
            }
            migrate_fn(&mut doc, &mut warnings);
        }
    }

    normalize_current_schema(&mut doc, &mut warnings);

    let migrated = doc.to_string();
    if migrated == raw {
        return Ok((raw.to_owned(), None));
    }

    let result = MigrationResult {
        from_version,
        to_version: read_version(&doc),
        warnings,
    };

    Ok((migrated, Some(result)))
}

/// Inspect the config for schema drift that would otherwise be ignored
/// silently by serde's permissive deserialization.
///
/// This returns schema issues for legacy aliases and active keys that are not
/// part of the current schema in selected high-risk sections.
pub fn inspect_config_issues(raw: &str) -> Result<Vec<String>, MigrationError> {
    let doc: DocumentMut = raw.parse()?;
    let mut warnings = Vec::new();

    inspect_agents_warnings(&doc, &mut warnings);
    inspect_auth_warnings(&doc, &mut warnings);
    inspect_llm_warnings(&doc, &mut warnings);
    inspect_tools_warnings(&doc, &mut warnings);
    inspect_observe_warnings(&doc, &mut warnings);
    inspect_adapter_warnings(&doc, &mut warnings);
    inspect_mcp_warnings(&doc, &mut warnings);
    inspect_a2a_warnings(&doc, &mut warnings);
    inspect_os_warnings(&doc, &mut warnings);
    inspect_http_warnings(&doc, &mut warnings);
    inspect_scheduler_warnings(&doc, &mut warnings);
    inspect_plugins_warnings(&doc, &mut warnings);

    Ok(warnings)
}

// ---------------------------------------------------------------------------
// Migration steps
// ---------------------------------------------------------------------------

/// v0 → v1: insert `config_version = 1` at the top of the document.
fn migrate_v0_to_v1(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    // Insert config_version as the first key by using toml_edit's decor API.
    // We set it and then move it to position 0 so it appears before other keys.
    doc.insert("config_version", toml_edit::value(1i64));

    // Move config_version to the front of the document.
    // toml_edit's Table doesn't have a move-to-front API, so we remove + re-insert
    // all other keys after it. Instead, we just set it — it will appear at the
    // insertion point. For a cleaner result, we accept it at the current position.

    warnings.push(
        "config_version was missing (legacy v0); set to 1. \
         Run `orka config migrate` to persist this change."
            .into(),
    );
}

/// v1 → v2: add `[agent]`, `[tools]`, and `[os.sudo]` sections with defaults
/// if they are absent. Also bumps `config_version` to 2.
///
/// The inserted defaults follow the current config schema so migrations do not
/// reintroduce stale keys that are ignored by deserialization.
fn migrate_v1_to_v2(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let mut added = Vec::new();

    if doc.get("agent").is_none() {
        let mut agent = Table::new();
        agent.insert("id", value("orka-default"));
        agent.insert("name", value("Orka"));
        agent.insert("max_turns", value(15i64));
        doc.insert("agent", Item::Table(agent));
        added.push("[agent]");
    }

    if doc.get("tools").is_none() {
        let mut tools = Table::new();
        let mut disabled = Array::new();
        disabled.push("echo");
        tools.insert("deny", value(disabled));
        doc.insert("tools", Item::Table(tools));
        added.push("[tools]");
    }

    // Insert [os.sudo] only if [os] already exists but [os.sudo] does not.
    if let Some(os_item) = doc.get_mut("os")
        && let Some(os_table) = os_item.as_table_mut()
        && os_table.get("sudo").is_none()
    {
        let mut sudo = Table::new();
        sudo.set_implicit(false);
        sudo.insert("allowed", value(false));
        let mut allowed = Array::new();
        allowed.push("systemctl restart");
        allowed.push("systemctl stop");
        allowed.push("systemctl start");
        sudo.insert("allowed_commands", value(allowed));
        sudo.insert("password_required", value(true));
        os_table.insert("sudo", Item::Table(sudo));
        added.push("[os.sudo]");
    }

    doc.insert("config_version", value(2i64));

    if !added.is_empty() {
        warnings.push(format!(
            "Added {} sections with defaults. Review and adjust as needed.",
            added.join(", ")
        ));
    }
}

/// v2 → v3: schema version bump only.
///
/// Earlier migration logic converted `os.claude_code.enabled` to a string
/// tri-state, but the current config schema uses a plain boolean. Keep the
/// value unchanged and only update the version marker.
fn migrate_v2_to_v3(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let _ = warnings;
    doc.insert("config_version", value(3i64));
}

/// v3 -> v4: normalize renamed keys and remove obsolete active settings so the
/// document matches the current strict schema.
fn migrate_v3_to_v4(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    migrate_v3_agent_keys(doc, warnings);
    migrate_v3_auth_keys(doc, warnings);
    migrate_v3_tools_keys(doc, warnings);
    migrate_v3_llm_keys(doc, warnings);
    migrate_v3_observe_keys(doc, warnings);
    migrate_v3_os_keys(doc, warnings);
    migrate_v3_http_keys(doc, warnings);
    migrate_v3_scheduler_keys(doc, warnings);
    doc.insert("config_version", value(4i64));
}

fn migrate_v3_agent_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(agent) = doc.get_mut("agent").and_then(Item::as_table_mut) else {
        return;
    };
    rename_key(
        agent,
        "display_name",
        "name",
        "[agent].display_name",
        "[agent].name",
        warnings,
    );
    rename_key(
        agent,
        "max_tool_result_chars",
        "tool_result_max_chars",
        "[agent].max_tool_result_chars",
        "[agent].tool_result_max_chars",
        warnings,
    );
    remove_keys(
        agent,
        &[
            ("timezone", "[agent].timezone"),
            ("heartbeat_interval_secs", "[agent].heartbeat_interval_secs"),
        ],
        warnings,
    );
}

fn migrate_v3_auth_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(auth) = doc.get_mut("auth").and_then(Item::as_table_mut) else {
        return;
    };
    remove_keys(
        auth,
        &[
            ("enabled", "[auth].enabled"),
            ("api_key_header", "[auth].api_key_header"),
        ],
        warnings,
    );
}

fn migrate_v3_tools_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(tools) = doc.get_mut("tools").and_then(Item::as_table_mut) else {
        return;
    };
    rename_key(
        tools,
        "disabled",
        "deny",
        "[tools].disabled",
        "[tools].deny",
        warnings,
    );
}

fn migrate_v3_llm_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(llm) = doc.get_mut("llm").and_then(Item::as_table_mut) else {
        return;
    };
    rename_key(
        llm,
        "model",
        "default_model",
        "[llm].model",
        "[llm].default_model",
        warnings,
    );
    rename_key(
        llm,
        "max_tokens",
        "default_max_tokens",
        "[llm].max_tokens",
        "[llm].default_max_tokens",
        warnings,
    );
    rename_key(
        llm,
        "temperature",
        "default_temperature",
        "[llm].temperature",
        "[llm].default_temperature",
        warnings,
    );
    remove_keys(llm, &[("timeout_secs", "[llm].timeout_secs")], warnings);
}

fn migrate_v3_observe_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(observe) = doc.get_mut("observe").and_then(Item::as_table_mut) else {
        return;
    };
    let Some(item) = observe.get_mut("backend") else {
        return;
    };
    let Some(backend) = item.as_str() else {
        return;
    };
    match backend {
        "log" => {
            *item = value("stdout");
            warnings
                .push("Migrated [observe].backend from legacy value \"log\" to \"stdout\".".into());
        }
        "otel" => {
            *item = value("otlp");
            warnings
                .push("Migrated [observe].backend from legacy value \"otel\" to \"otlp\".".into());
        }
        _ => {}
    }
}

fn migrate_v3_os_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(os) = doc.get_mut("os").and_then(Item::as_table_mut) else {
        return;
    };
    rename_key(
        os,
        "blocked_paths",
        "denied_paths",
        "[os].blocked_paths",
        "[os].denied_paths",
        warnings,
    );
    rename_key(
        os,
        "allowed_commands",
        "allowed_shell_commands",
        "[os].allowed_commands",
        "[os].allowed_shell_commands",
        warnings,
    );
    remove_keys(
        os,
        &[
            ("shell_timeout_secs", "[os].shell_timeout_secs"),
            ("max_output_bytes", "[os].max_output_bytes"),
            ("max_file_size_bytes", "[os].max_file_size_bytes"),
            ("max_list_entries", "[os].max_list_entries"),
            ("blocked_commands", "[os].blocked_commands"),
            ("sensitive_env_patterns", "[os].sensitive_env_patterns"),
        ],
        warnings,
    );
    if let Some(sudo) = os.get_mut("sudo").and_then(Item::as_table_mut) {
        rename_key(
            sudo,
            "enabled",
            "allowed",
            "[os.sudo].enabled",
            "[os.sudo].allowed",
            warnings,
        );
        remove_keys(
            sudo,
            &[
                ("require_confirmation", "[os.sudo].require_confirmation"),
                (
                    "confirmation_timeout_secs",
                    "[os.sudo].confirmation_timeout_secs",
                ),
                ("sudo_path", "[os.sudo].sudo_path"),
            ],
            warnings,
        );
    }
}

fn migrate_v3_http_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(http) = doc.get_mut("http").and_then(Item::as_table_mut) else {
        return;
    };
    remove_keys(
        http,
        &[
            ("enabled", "[http].enabled"),
            ("max_response_bytes", "[http].max_response_bytes"),
            ("default_timeout_secs", "[http].default_timeout_secs"),
            ("blocked_domains", "[http].blocked_domains"),
        ],
        warnings,
    );
}

fn migrate_v3_scheduler_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(scheduler) = doc.get_mut("scheduler").and_then(Item::as_table_mut) else {
        return;
    };
    remove_keys(
        scheduler,
        &[
            ("poll_interval_secs", "[scheduler].poll_interval_secs"),
            ("max_concurrent", "[scheduler].max_concurrent"),
        ],
        warnings,
    );
}

/// v4 → v5: promote `[agent]` to `[[agents]]` + `[graph]`.
///
/// The single-agent shorthand `[agent]` is now sugar for a one-entry
/// `[[agents]]` array. This migration converts any existing `[agent]` table
/// to the canonical form so the rest of the system always works through the
/// unified multi-agent graph path.
fn migrate_v4_to_v5(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let has_agent = doc.get("agent").and_then(Item::as_table).is_some();
    let has_agents = doc
        .get("agents")
        .and_then(Item::as_array_of_tables)
        .is_some_and(|a| !a.is_empty());

    if has_agent && !has_agents {
        // Collect all fields from [agent] so we can rebuild them in [[agents]].
        let agent_fields: Vec<(String, Item)> = doc
            .get("agent")
            .and_then(Item::as_table)
            .into_iter()
            .flat_map(|t| t.iter().map(|(k, v)| (k.to_string(), v.clone())))
            .collect();

        let mut entry = Table::new();
        for (key, item) in &agent_fields {
            entry.insert(key, item.clone());
        }
        // `id` is required on every [[agents]] entry.
        if !entry.contains_key("id") {
            entry.insert("id", value("orka-default"));
        }

        let mut aot = ArrayOfTables::new();
        aot.push(entry);
        doc.insert("agents", Item::ArrayOfTables(aot));

        // Auto-generate an empty [graph] (no edges = single-node, standalone).
        if doc.get("graph").is_none() {
            doc.insert("graph", Item::Table(Table::new()));
        }

        doc.remove("agent");

        warnings.push(
            "Migrated [agent] to [[agents]] + [graph] (unified graph path). \
             Run `orka config migrate` to persist this change."
                .into(),
        );
    }
    // If [[agents]] already present (with or without a stale [agent]): keep as-is.

    doc.insert("config_version", value(5i64));
}

/// v5 → v6: remove the `subprocess` capability from `[plugins.capabilities]`
/// (WASI Component Model does not support subprocess spawning) and normalize
/// `filesystem = true/false` to an array for clarity.
fn migrate_v5_to_v6(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    // Navigate to [plugins.capabilities] if it exists.
    let has_subprocess = doc
        .get("plugins")
        .and_then(|p| p.get("capabilities"))
        .and_then(|c| c.get("subprocess"))
        .is_some();

    if has_subprocess {
        if let Some(caps) = doc
            .get_mut("plugins")
            .and_then(|p| p.as_table_mut())
            .and_then(|t| t.get_mut("capabilities"))
            .and_then(|c| c.as_table_mut())
        {
            caps.remove("subprocess");
        }
        warnings.push(
            "Removed `plugins.capabilities.subprocess`: WASI Component Model does not support \
             subprocess spawning. Remove this key from your config to silence this warning."
                .into(),
        );
    }

    doc.insert("config_version", value(6i64));
}

/// Inspect `[[agents]]` entries for unknown or legacy keys.
/// Also warns if the legacy `[agent]` table is still present (should have been
/// promoted by the v4→v5 migration).
fn inspect_agents_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    // Warn if the legacy [agent] single-table is still present (migration missed
    // it).
    if doc.get("agent").and_then(Item::as_table).is_some() {
        warnings.push(
            "config key [agent] is legacy as of v5; it should have been promoted to [[agents]] \
             by the migration. Run `orka config migrate` to fix this."
                .into(),
        );
    }

    let entry_allowed = [
        "id",
        "name",
        "system_prompt",
        "model",
        "temperature",
        "max_tokens",
        "thinking",
        "max_turns",
        "tool_result_max_chars",
        "allowed_tools",
        "denied_tools",
        // Added in multi-agent graph improvements
        "kind",
        "history_filter",
        "history_filter_n",
        // Added in Phase 2
        "planning_mode",
        "history_strategy",
        "interrupt_before_tools",
        // Per-skill execution limits
        "skill_timeout_secs",
        "max_concurrent_skills",
    ];
    let known: BTreeSet<&str> = entry_allowed.iter().copied().collect();

    let Some(agents) = doc.get("agents").and_then(Item::as_array_of_tables) else {
        return;
    };

    for (idx, table) in agents.iter().enumerate() {
        for (key, _) in table {
            if !known.contains(key) {
                warnings.push(format!(
                    "config key [[agents]][{idx}].{key} is unknown to the current schema and may be ignored"
                ));
            }
        }
    }
}

fn inspect_auth_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = ["jwt", "api_keys", "token_url", "auth_url"];
    let aliases: [(&str, &str); 0] = [];
    let removed = [
        (
            "enabled",
            "config key [auth].enabled is not part of the current schema and may be ignored",
        ),
        (
            "api_key_header",
            "config key [auth].api_key_header is not part of the current schema and may be ignored",
        ),
    ];

    inspect_table_keys(doc, &["auth"], &allowed, &aliases, &removed, warnings);
}

fn inspect_llm_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = [
        "default_model",
        "default_temperature",
        "default_max_tokens",
        "providers",
    ];
    let aliases = [
        (
            "model",
            "config key [llm].model is legacy; use [llm].default_model",
        ),
        (
            "max_tokens",
            "config key [llm].max_tokens is legacy; use [llm].default_max_tokens",
        ),
        (
            "temperature",
            "config key [llm].temperature is legacy; use [llm].default_temperature",
        ),
    ];
    let removed = [(
        "timeout_secs",
        "config key [llm].timeout_secs is not part of the current schema and may be ignored",
    )];

    inspect_table_keys(doc, &["llm"], &allowed, &aliases, &removed, warnings);

    if let Some(providers) = doc
        .get("llm")
        .and_then(|item| item.get("providers"))
        .and_then(Item::as_array_of_tables)
    {
        let provider_allowed = [
            "name",
            "provider",
            "base_url",
            "model",
            "api_key",
            "api_key_env",
            "api_key_secret",
            "temperature",
            "max_tokens",
            "top_p",
            "timeout_secs",
            "max_retries",
        ];
        for (idx, table) in providers.iter().enumerate() {
            if table.contains_key("prefixes") {
                warnings.push(format!(
                    "config key [[llm.providers]][{idx}].prefixes is not part of the current schema and may be ignored"
                ));
            }
            let known: BTreeSet<&str> = provider_allowed.iter().copied().collect();
            for (key, _) in table {
                if !known.contains(key) && key != "prefixes" {
                    warnings.push(format!(
                        "config key [[llm.providers]][{idx}].{key} is unknown to the current schema and may be ignored"
                    ));
                }
            }
        }
    }
}

fn inspect_tools_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = ["allow", "deny", "config"];
    let aliases = [(
        "disabled",
        "config key [tools].disabled is legacy; use [tools].deny",
    )];
    let removed: [(&str, &str); 0] = [];

    inspect_table_keys(doc, &["tools"], &allowed, &aliases, &removed, warnings);
}

fn inspect_observe_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = [
        "enabled",
        "backend",
        "otlp_endpoint",
        "batch_size",
        "flush_interval_ms",
        "service_name",
        "service_version",
    ];
    let aliases: [(&str, &str); 0] = [];
    let removed: [(&str, &str); 0] = [];
    inspect_table_keys(doc, &["observe"], &allowed, &aliases, &removed, warnings);

    if let Some(observe) = get_table(doc, &["observe"])
        && let Some(backend) = observe.get("backend").and_then(Item::as_str)
        && !matches!(backend, "stdout" | "prometheus" | "otlp")
    {
        warnings.push(format!(
            "config value [observe].backend = {backend:?} is not part of the current schema; use one of \"stdout\", \"prometheus\", or \"otlp\""
        ));
    }
}

fn inspect_adapter_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let telegram_allowed = [
        "bot_token_secret",
        "workspace",
        "mode",
        "webhook_url",
        "webhook_port",
        "parse_mode",
        "streaming",
    ];
    let telegram_removed = [
        (
            "owner_id",
            "config key [adapters.telegram].owner_id is not part of the current schema and may be ignored",
        ),
        (
            "allowed_users",
            "config key [adapters.telegram].allowed_users is not part of the current schema and may be ignored",
        ),
    ];
    inspect_table_keys(
        doc,
        &["adapters", "telegram"],
        &telegram_allowed,
        &[],
        &telegram_removed,
        warnings,
    );

    let discord_allowed = ["bot_token_secret", "workspace"];
    let discord_removed = [(
        "application_id",
        "config key [adapters.discord].application_id is not part of the current schema and may be ignored",
    )];
    inspect_table_keys(
        doc,
        &["adapters", "discord"],
        &discord_allowed,
        &[],
        &discord_removed,
        warnings,
    );

    let slack_allowed = [
        "bot_token_secret",
        "signing_secret_path",
        "workspace",
        "port",
    ];
    let slack_removed = [(
        "listen_port",
        "config key [adapters.slack].listen_port is not part of the current schema and may be ignored; use [adapters.slack].port",
    )];
    inspect_table_keys(
        doc,
        &["adapters", "slack"],
        &slack_allowed,
        &[],
        &slack_removed,
        warnings,
    );

    let whatsapp_allowed = [
        "access_token_secret",
        "app_secret_path",
        "phone_number_id",
        "business_account_id",
        "workspace",
        "port",
        "verify_token",
    ];
    let whatsapp_removed = [
        (
            "verify_token_secret",
            "config key [adapters.whatsapp].verify_token_secret is not part of the current schema and may be ignored; use [adapters.whatsapp].verify_token",
        ),
        (
            "listen_port",
            "config key [adapters.whatsapp].listen_port is not part of the current schema and may be ignored; use [adapters.whatsapp].port",
        ),
    ];
    inspect_table_keys(
        doc,
        &["adapters", "whatsapp"],
        &whatsapp_allowed,
        &[],
        &whatsapp_removed,
        warnings,
    );
}

fn inspect_mcp_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = ["servers", "client"];
    let removed = [(
        "serve",
        "config key [mcp].serve is not part of the current schema and may be ignored",
    )];
    inspect_table_keys(doc, &["mcp"], &allowed, &[], &removed, warnings);
}

fn inspect_a2a_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = [
        "discovery_enabled",
        "discovery_interval_secs",
        "known_agents",
    ];
    let removed = [
        (
            "enabled",
            "config key [a2a].enabled is not part of the current schema and may be ignored; use [a2a].discovery_enabled",
        ),
        (
            "url",
            "config key [a2a].url is not part of the current schema and may be ignored",
        ),
    ];
    inspect_table_keys(doc, &["a2a"], &allowed, &[], &removed, warnings);
}

fn inspect_os_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    inspect_os_base_keys(doc, warnings);
    inspect_os_coding_keys(doc, warnings);
    inspect_os_sudo_keys(doc, warnings);
}

fn inspect_os_base_keys(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = [
        "enabled",
        "permission_level",
        "allowed_paths",
        "denied_paths",
        "allowed_shell_commands",
        "coding",
        "sudo",
    ];
    let aliases = [(
        "blocked_paths",
        "config key [os].blocked_paths is legacy; use [os].denied_paths",
    )];
    let removed = [
        (
            "shell_timeout_secs",
            "config key [os].shell_timeout_secs is not part of the current schema and may be ignored",
        ),
        (
            "max_output_bytes",
            "config key [os].max_output_bytes is not part of the current schema and may be ignored",
        ),
        (
            "max_file_size_bytes",
            "config key [os].max_file_size_bytes is not part of the current schema and may be ignored",
        ),
        (
            "max_list_entries",
            "config key [os].max_list_entries is not part of the current schema and may be ignored",
        ),
        (
            "allowed_commands",
            "config key [os].allowed_commands is not part of the current schema and may be ignored; use [os].allowed_shell_commands when applicable",
        ),
        (
            "blocked_commands",
            "config key [os].blocked_commands is not part of the current schema and may be ignored",
        ),
        (
            "sensitive_env_patterns",
            "config key [os].sensitive_env_patterns is not part of the current schema and may be ignored",
        ),
    ];
    inspect_table_keys(doc, &["os"], &allowed, &aliases, &removed, warnings);
}

fn inspect_os_coding_keys(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let coding_allowed = [
        "enabled",
        "default_provider",
        "selection_policy",
        "inject_workspace_context",
        "require_verification",
        "allow_working_dir_override",
        "providers",
    ];
    inspect_table_keys(doc, &["os", "coding"], &coding_allowed, &[], &[], warnings);
    inspect_table_keys(
        doc,
        &["os", "coding", "providers"],
        &["claude_code", "codex", "opencode"],
        &[],
        &[],
        warnings,
    );

    let claude_allowed = [
        "enabled",
        "executable_path",
        "model",
        "max_turns",
        "timeout_secs",
        "append_system_prompt",
        "allowed_tools",
        "allow_file_modifications",
        "allow_command_execution",
    ];
    inspect_table_keys(
        doc,
        &["os", "coding", "providers", "claude_code"],
        &claude_allowed,
        &[],
        &[],
        warnings,
    );

    let codex_allowed = [
        "enabled",
        "executable_path",
        "model",
        "timeout_secs",
        "sandbox_mode",
        "approval_policy",
        "allow_file_modifications",
        "allow_command_execution",
    ];
    inspect_table_keys(
        doc,
        &["os", "coding", "providers", "codex"],
        &codex_allowed,
        &[],
        &[],
        warnings,
    );

    let opencode_allowed = [
        "enabled",
        "executable_path",
        "model",
        "agent",
        "variant",
        "timeout_secs",
        "allow_file_modifications",
        "allow_command_execution",
    ];
    inspect_table_keys(
        doc,
        &["os", "coding", "providers", "opencode"],
        &opencode_allowed,
        &[],
        &[],
        warnings,
    );
}

fn inspect_os_sudo_keys(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let sudo_allowed = ["allowed", "allowed_commands", "password_required"];
    let sudo_aliases = [(
        "enabled",
        "config key [os.sudo].enabled is legacy; use [os.sudo].allowed",
    )];
    let sudo_removed = [
        (
            "require_confirmation",
            "config key [os.sudo].require_confirmation is not part of the current schema and may be ignored",
        ),
        (
            "confirmation_timeout_secs",
            "config key [os.sudo].confirmation_timeout_secs is not part of the current schema and may be ignored",
        ),
        (
            "sudo_path",
            "config key [os.sudo].sudo_path is not part of the current schema and may be ignored",
        ),
    ];
    inspect_table_keys(
        doc,
        &["os", "sudo"],
        &sudo_allowed,
        &sudo_aliases,
        &sudo_removed,
        warnings,
    );
}

fn inspect_http_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = [
        "timeout_secs",
        "max_redirects",
        "user_agent",
        "default_headers",
        "webhooks",
    ];
    let aliases: [(&str, &str); 0] = [];
    let removed = [
        (
            "enabled",
            "config key [http].enabled is not part of the current schema and may be ignored",
        ),
        (
            "max_response_bytes",
            "config key [http].max_response_bytes is not part of the current schema and may be ignored",
        ),
        (
            "default_timeout_secs",
            "config key [http].default_timeout_secs is not part of the current schema and may be ignored; use [http].timeout_secs",
        ),
        (
            "blocked_domains",
            "config key [http].blocked_domains is not part of the current schema and may be ignored",
        ),
    ];
    inspect_table_keys(doc, &["http"], &allowed, &aliases, &removed, warnings);
}

fn inspect_scheduler_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    let allowed = ["enabled", "jobs"];
    let aliases: [(&str, &str); 0] = [];
    let removed = [
        (
            "poll_interval_secs",
            "config key [scheduler].poll_interval_secs is not part of the current schema and may be ignored",
        ),
        (
            "max_concurrent",
            "config key [scheduler].max_concurrent is not part of the current schema and may be ignored",
        ),
    ];
    inspect_table_keys(doc, &["scheduler"], &allowed, &aliases, &removed, warnings);
}

fn inspect_plugins_warnings(doc: &DocumentMut, warnings: &mut Vec<String>) {
    // Top-level [plugins] keys.
    let allowed = ["dir", "capabilities", "plugins"];
    let aliases: [(&str, &str); 0] = [];
    let removed = [(
        "subprocess",
        "config key [plugins].subprocess is not part of the current schema and may be ignored",
    )];
    inspect_table_keys(doc, &["plugins"], &allowed, &aliases, &removed, warnings);

    // [plugins.capabilities] keys (v5 had `subprocess` here).
    let caps_allowed = ["filesystem", "network", "env"];
    let caps_removed = [(
        "subprocess",
        "config key [plugins.capabilities].subprocess was removed in v6 \
         (WASI Component Model does not support subprocess spawning)",
    )];
    inspect_table_keys(
        doc,
        &["plugins", "capabilities"],
        &caps_allowed,
        &[],
        &caps_removed,
        warnings,
    );
}

fn normalize_current_schema(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    normalize_agents_keys(doc, warnings);
}

fn normalize_agents_keys(doc: &mut DocumentMut, warnings: &mut Vec<String>) {
    let Some(agents) = doc.get_mut("agents").and_then(Item::as_array_of_tables_mut) else {
        return;
    };

    for (idx, table) in agents.iter_mut().enumerate() {
        let legacy_label = format!("[[agents]][{idx}].max_iterations");
        let current_label = format!("[[agents]][{idx}].max_turns");
        rename_key(
            table,
            "max_iterations",
            "max_turns",
            &legacy_label,
            &current_label,
            warnings,
        );
    }
}

fn rename_key(
    table: &mut Table,
    legacy_key: &str,
    current_key: &str,
    legacy_label: &str,
    current_label: &str,
    warnings: &mut Vec<String>,
) {
    if !table.contains_key(legacy_key) {
        return;
    }

    if table.contains_key(current_key) {
        table.remove(legacy_key);
        warnings.push(format!(
            "Removed legacy key {legacy_label} because {current_label} is already set."
        ));
        return;
    }

    if let Some(item) = table.remove(legacy_key) {
        table.insert(current_key, item);
        warnings.push(format!("Migrated {legacy_label} to {current_label}."));
    }
}

fn remove_keys(table: &mut Table, keys: &[(&str, &str)], warnings: &mut Vec<String>) {
    for (key, label) in keys {
        if table.remove(key).is_some() {
            warnings.push(format!("Removed obsolete config key {label}."));
        }
    }
}

fn inspect_table_keys(
    doc: &DocumentMut,
    path: &[&str],
    allowed: &[&str],
    aliases: &[(&str, &str)],
    removed: &[(&str, &str)],
    warnings: &mut Vec<String>,
) {
    let Some(table) = get_table(doc, path) else {
        return;
    };

    let allowed: BTreeSet<&str> = allowed.iter().copied().collect();
    for (legacy_key, message) in aliases {
        if table.contains_key(legacy_key) {
            warnings.push((*message).to_string());
        }
    }
    for (removed_key, message) in removed {
        if table.contains_key(removed_key) {
            warnings.push((*message).to_string());
        }
    }

    let known: BTreeSet<&str> = allowed
        .iter()
        .copied()
        .chain(aliases.iter().map(|(key, _)| *key))
        .chain(removed.iter().map(|(key, _)| *key))
        .collect();

    let path_label = format!("[{}]", path.join("."));
    for (key, _) in table {
        if !known.contains(key) {
            warnings.push(format!(
                "config key {path_label}.{key} is unknown to the current schema and may be ignored"
            ));
        }
    }
}

fn get_table<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a Table> {
    let mut current = doc.as_item();
    for segment in path {
        current = current.get(segment)?;
    }
    current.as_table()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
        result.unwrap_or_else(|err| panic!("unexpected error: {err}"))
    }

    fn some<T>(value: Option<T>, msg: &str) -> T {
        value.unwrap_or_else(|| panic!("{msg}"))
    }

    fn err<T, E>(result: std::result::Result<T, E>) -> E {
        match result {
            Ok(_) => panic!("expected error"),
            Err(err) => err,
        }
    }

    #[test]
    fn missing_version_treated_as_v0_and_migrated() {
        let raw = r#"
[server]
host = "127.0.0.1"
port = 8080
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 0);
        assert_eq!(result.to_version, 6);
        assert!(!result.warnings.is_empty());

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
    }

    #[test]
    fn current_version_is_noop() {
        let raw = r#"
config_version = 6

[server]
host = "127.0.0.1"
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        assert!(result.is_none());
        assert_eq!(migrated, raw);
    }

    #[test]
    fn current_version_normalizes_legacy_agent_keys() {
        let raw = r#"
config_version = 6

[[agents]]
id = "orka-default"
max_iterations = 25

[graph]
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have normalization result");
        assert_eq!(result.from_version, 6);
        assert_eq!(result.to_version, 6);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("[[agents]][0].max_iterations"))
        );

        let doc: DocumentMut = ok(migrated.parse());
        let agents = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .unwrap_or_else(|| panic!("[[agents]] missing"));
        let agent = some(agents.iter().next(), "at least one agent");
        assert_eq!(agent["max_turns"].as_integer(), Some(25));
        assert!(agent.get("max_iterations").is_none());
    }

    #[test]
    fn current_version_prefers_max_turns_over_legacy_max_iterations() {
        let raw = r#"
config_version = 6

[[agents]]
id = "orka-default"
max_iterations = 25
max_turns = 10

[graph]
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have normalization result");
        assert_eq!(result.from_version, 6);
        assert_eq!(result.to_version, 6);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("max_turns is already set"))
        );

        let doc: DocumentMut = ok(migrated.parse());
        let agents = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .unwrap_or_else(|| panic!("[[agents]] missing"));
        let agent = some(agents.iter().next(), "at least one agent");
        assert_eq!(agent["max_turns"].as_integer(), Some(10));
        assert!(agent.get("max_iterations").is_none());
    }

    #[test]
    fn future_version_rejected() {
        let raw = "config_version = 999\n";
        let err = err(migrate_if_needed(raw));
        assert!(matches!(
            err,
            MigrationError::FutureVersion { found: 999, max: 6 }
        ));
        let msg = err.to_string();
        assert!(msg.contains("999"));
    }

    #[test]
    fn v1_to_v2_adds_missing_sections() {
        let raw = r#"config_version = 1

[server]
host = "127.0.0.1"
port = 8080

[os]
enabled = true
permission_level = "read-only"
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 1);
        assert_eq!(result.to_version, 6);
        // v1→v2 emits 1 warning (added [agent]+[tools]+[os.sudo]); v4→v5 adds 1 more
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("[agent]"));
        assert!(result.warnings[0].contains("[tools]"));
        assert!(result.warnings[0].contains("[os.sudo]"));

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);

        // [[agents]] promoted from legacy [agent]
        let agents = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .unwrap_or_else(|| panic!("[[agents]] missing"));
        let agent = some(agents.iter().next(), "at least one agent");
        assert_eq!(agent["id"].as_str(), Some("orka-default"));
        assert_eq!(agent["name"].as_str(), Some("Orka"));
        assert_eq!(agent["max_turns"].as_integer(), Some(15));

        // [tools] defaults
        let tools = doc["tools"]
            .as_table()
            .unwrap_or_else(|| panic!("[tools] missing"));
        let disabled = tools["deny"]
            .as_array()
            .unwrap_or_else(|| panic!("disabled array missing"));
        assert_eq!(
            disabled.iter().next().and_then(|v| v.as_str()),
            Some("echo")
        );

        // [os.sudo] defaults
        let os = doc["os"]
            .as_table()
            .unwrap_or_else(|| panic!("[os] missing"));
        let sudo = os["sudo"]
            .as_table()
            .unwrap_or_else(|| panic!("[os.sudo] missing"));
        assert_eq!(sudo["allowed"].as_bool(), Some(false));
        assert_eq!(sudo["password_required"].as_bool(), Some(true));
    }

    #[test]
    fn v1_to_v2_preserves_existing_sections() {
        let raw = r#"config_version = 1

[agent]
id = "my-agent"
name = "MyBot"
max_turns = 5

[tools]
deny = []

[os]
enabled = true

[os.sudo]
allowed = true
allowed_commands = ["apt-get install"]
password_required = false
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 1);
        assert_eq!(result.to_version, 6);
        // v4→v5 emits one migration warning (promoting [agent] → [[agents]])
        assert!(
            result
                .warnings
                .iter()
                .all(|w| w.contains("Migrated [agent]")),
            "unexpected warnings: {:?}",
            result.warnings
        );

        let doc: DocumentMut = ok(migrated.parse());
        // Existing values preserved in promoted [[agents]]
        let agents = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .unwrap_or_else(|| panic!("[[agents]] missing"));
        let first = some(agents.iter().next(), "at least one agent");
        assert_eq!(first["id"].as_str(), Some("my-agent"));
        assert_eq!(first["max_turns"].as_integer(), Some(5));
        let os = doc["os"]
            .as_table()
            .unwrap_or_else(|| panic!("[os] missing"));
        let sudo = os["sudo"]
            .as_table()
            .unwrap_or_else(|| panic!("[os.sudo] missing"));
        assert_eq!(sudo["allowed"].as_bool(), Some(true));
    }

    #[test]
    fn v1_to_v2_no_os_section_skips_sudo() {
        let raw = "config_version = 1\n\n[server]\nhost = \"127.0.0.1\"\n";
        let (migrated, _) = ok(migrate_if_needed(raw));
        let doc: DocumentMut = ok(migrated.parse());
        assert!(doc.get("os").is_none(), "[os] should not be inserted");
    }

    #[test]
    fn comments_preserved_after_migration() {
        let raw = r#"# This is my config
[server]
host = "127.0.0.1" # inline comment
port = 8080
"#;
        let (migrated, _) = ok(migrate_if_needed(raw));
        assert!(migrated.contains("# This is my config"));
        assert!(migrated.contains("# inline comment"));
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let raw = "config_version = [invalid";
        let err = err(migrate_if_needed(raw));
        assert!(matches!(err, MigrationError::Parse(_)));
    }

    #[test]
    fn v2_to_v3_converts_enabled_true() {
        let raw = r"config_version = 2

[os.claude_code]
enabled = true
";
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 2);
        assert_eq!(result.to_version, 6);
        assert!(result.warnings.is_empty());

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
        assert_eq!(doc["os"]["claude_code"]["enabled"].as_bool(), Some(true));
    }

    #[test]
    fn v2_to_v3_converts_enabled_false() {
        let raw = r"config_version = 2

[os.claude_code]
enabled = false
";
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 2);
        assert_eq!(result.to_version, 6);
        assert!(result.warnings.is_empty());

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
        assert_eq!(doc["os"]["claude_code"]["enabled"].as_bool(), Some(false));
    }

    #[test]
    fn v2_to_v3_no_claude_code_section_is_noop() {
        let raw = r#"config_version = 2

[server]
host = "127.0.0.1"
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 2);
        assert_eq!(result.to_version, 6);
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
    }

    #[test]
    fn v2_to_v3_string_enabled_unchanged() {
        // If someone already has enabled = "auto", it should be left as-is.
        let raw = r#"config_version = 2

[os.claude_code]
enabled = "auto"
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(doc["os"]["claude_code"]["enabled"].as_str(), Some("auto"));
    }

    #[test]
    fn v0_to_v3_chain() {
        let raw = r#"
[server]
host = "127.0.0.1"
port = 8080
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 0);
        assert_eq!(result.to_version, 6);

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
        // v1→v2 adds [tools]; v4→v5 promotes [agent] → [[agents]]
        assert!(doc.get("agents").is_some());
        assert!(doc.get("tools").is_some());
    }

    #[test]
    fn inspect_config_issues_reports_legacy_and_unknown_keys() {
        // config_version = 5, [[agents]] with unknown fields (display_name,
        // some_bogus_field)
        let raw = r#"
config_version = 5

[[agents]]
id = "orka-default"
display_name = "Orka"
some_bogus_field = true

[graph]

[auth]
enabled = false
api_key_header = "X-Api-Key"

[adapters.telegram]
bot_token_secret = "telegram"
owner_id = 123

[tools]
disabled = ["echo"]

[llm]
max_tokens = 8192

[[llm.providers]]
name = "anthropic"
provider = "anthropic"
prefixes = ["claude"]

[os]
enabled = true
permission_level = "admin"
shell_timeout_secs = 120

[os.sudo]
enabled = true
require_confirmation = true
"#;
        let warnings = ok(inspect_config_issues(raw));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("[[agents]][0].display_name")),
            "missing [[agents]][0].display_name warning; got: {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("[[agents]][0].some_bogus_field")),
            "missing [[agents]][0].some_bogus_field warning; got: {warnings:?}"
        );
        assert!(warnings.iter().any(|w| w.contains("[auth].enabled")));
        assert!(warnings.iter().any(|w| w.contains("[auth].api_key_header")));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("[adapters.telegram].owner_id"))
        );
        assert!(warnings.iter().any(|w| w.contains("[tools].disabled")));
        assert!(warnings.iter().any(|w| w.contains("[llm].max_tokens")));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("[[llm.providers]][0].prefixes"))
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("[os].shell_timeout_secs"))
        );
        assert!(warnings.iter().any(|w| w.contains("[os.sudo].enabled")));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("[os.sudo].require_confirmation"))
        );
    }

    #[test]
    fn inspect_config_issues_is_empty_for_current_keys() {
        let raw = r#"
config_version = 5

[[agents]]
id = "orka-default"
name = "Orka"
model = "claude-sonnet-4-6"
max_tokens = 8192
max_turns = 10
tool_result_max_chars = 1000
thinking = "high"

[graph]

[tools]
deny = ["echo"]

[llm]
default_model = "claude-sonnet-4-6"
default_temperature = 0.7
default_max_tokens = 8192

[os]
enabled = true
permission_level = "admin"
allowed_paths = ["/var/lib/orka"]

[os.sudo]
allowed = true
allowed_commands = ["systemctl restart"]
password_required = true
"#;
        let warnings = ok(inspect_config_issues(raw));
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn v3_to_v4_renames_and_removes_legacy_keys() {
        let raw = r#"config_version = 3

[agent]
id = "orka-default"
display_name = "Orka"
thinking_budget_tokens = 10000

[auth]
enabled = false
api_key_header = "X-Api-Key"

[tools]
disabled = ["echo"]

[llm]
model = "claude-sonnet-4-6"
max_tokens = 8192
temperature = 0.7
timeout_secs = 120

[observe]
backend = "log"

[os]
enabled = true
permission_level = "admin"
allowed_commands = ["git status"]
shell_timeout_secs = 120

[os.sudo]
enabled = true
require_confirmation = true

[http]
enabled = true
max_response_bytes = 1048576

[scheduler]
enabled = true
poll_interval_secs = 5
max_concurrent = 4
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 3);
        assert_eq!(result.to_version, 6);

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
        // v3→v4 cleaned [agent] fields; v4→v5 promoted [agent] → [[agents]]
        let agents = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .unwrap_or_else(|| panic!("[[agents]] missing"));
        let agent = some(agents.iter().next(), "at least one agent");
        assert_eq!(agent["name"].as_str(), Some("Orka"));
        assert!(agent.get("display_name").is_none());
        assert_eq!(agent["thinking_budget_tokens"].as_integer(), Some(10000));
        assert!(doc["auth"].get("enabled").is_none());
        assert!(doc["auth"].get("api_key_header").is_none());
        assert!(doc["tools"].get("disabled").is_none());
        assert!(doc["tools"]["deny"].as_array().is_some());
        assert_eq!(
            doc["llm"]["default_model"].as_str(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(doc["llm"]["default_max_tokens"].as_integer(), Some(8192));
        assert_eq!(doc["llm"]["default_temperature"].as_float(), Some(0.7));
        assert!(doc["llm"].get("timeout_secs").is_none());
        assert_eq!(doc["observe"]["backend"].as_str(), Some("stdout"));
        assert_eq!(
            doc["os"]["allowed_shell_commands"]
                .as_array()
                .and_then(|arr| arr.iter().next())
                .and_then(|v| v.as_str()),
            Some("git status")
        );
        assert!(doc["os"].get("allowed_commands").is_none());
        assert!(doc["os"].get("shell_timeout_secs").is_none());
        assert_eq!(doc["os"]["sudo"]["allowed"].as_bool(), Some(true));
        assert!(doc["os"]["sudo"].get("enabled").is_none());
        assert!(doc["os"]["sudo"].get("require_confirmation").is_none());
        assert!(doc["http"].get("enabled").is_none());
        assert!(doc["http"].get("max_response_bytes").is_none());
        assert!(doc["scheduler"].get("poll_interval_secs").is_none());
        assert!(doc["scheduler"].get("max_concurrent").is_none());
    }

    #[test]
    fn v5_to_v6_strips_subprocess() {
        let raw = r#"config_version = 5

[plugins]
dir = "plugins"

[plugins.capabilities]
filesystem = true
network = false
subprocess = true
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 5);
        assert_eq!(result.to_version, 6);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("subprocess"));

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
        let caps = doc["plugins"]["capabilities"]
            .as_table()
            .unwrap_or_else(|| panic!("[plugins.capabilities] missing"));
        assert!(
            caps.get("subprocess").is_none(),
            "subprocess should be removed"
        );
        assert!(caps.get("filesystem").is_some(), "filesystem preserved");
        assert!(caps.get("network").is_some(), "network preserved");
    }

    #[test]
    fn v5_to_v6_noop_without_subprocess() {
        let raw = r#"config_version = 5

[plugins]
dir = "plugins"

[plugins.capabilities]
filesystem = false
network = false
"#;
        let (migrated, result) = ok(migrate_if_needed(raw));
        let result = some(result, "should have migration result");
        assert_eq!(result.from_version, 5);
        assert_eq!(result.to_version, 6);
        assert!(
            result.warnings.is_empty(),
            "no warnings expected: {:?}",
            result.warnings
        );

        let doc: DocumentMut = ok(migrated.parse());
        assert_eq!(read_version(&doc), 6);
    }

    #[test]
    fn inspect_plugins_warns_on_subprocess() {
        let raw = r"
config_version = 6

[plugins.capabilities]
subprocess = true
";
        let warnings = ok(inspect_config_issues(raw));
        assert!(
            warnings.iter().any(|w| w.contains("subprocess")),
            "expected subprocess warning; got: {warnings:?}"
        );
    }
}
