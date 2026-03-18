//! Config versioning and migration engine.
//!
//! Each config version is an integer. Migrations run sequentially (v0→v1→v2→…)
//! using `toml_edit` to preserve comments and formatting. The server migrates
//! in-memory at boot; only the CLI writes the file.

use toml_edit::{Array, DocumentMut, Item, Table, value};

/// The config version that this build of Orka expects.
pub const CURRENT_CONFIG_VERSION: u32 = 3;

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

    /// A migration step failed.
    #[error("migration from v{from} to v{to} failed: {reason}")]
    StepFailed {
        /// Source version of the failing step.
        from: u32,
        /// Target version of the failing step.
        to: u32,
        /// Human-readable reason.
        reason: String,
    },
}

/// Signature for a single migration step.
///
/// Receives a mutable TOML document and a warnings collector.
/// Must update `config_version` in the document to `target_version`.
type MigrateFn = fn(&mut DocumentMut, &mut Vec<String>) -> Result<(), String>;

/// Registry of migrations. Each entry is `(target_version, migrate_fn)`.
/// The function migrates from `target_version - 1` to `target_version`.
const MIGRATIONS: &[(u32, MigrateFn)] = &[
    (1, migrate_v0_to_v1),
    (2, migrate_v1_to_v2),
    (3, migrate_v2_to_v3),
];

/// Read `config_version` from a parsed TOML document. Returns 0 if absent.
fn read_version(doc: &DocumentMut) -> u32 {
    doc.get("config_version")
        .and_then(|v| v.as_integer())
        .map(|v| v as u32)
        .unwrap_or(0)
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

    // Already current — nothing to do.
    if from_version == CURRENT_CONFIG_VERSION {
        return Ok((raw.to_owned(), None));
    }

    let mut warnings = Vec::new();

    // Apply each step sequentially.
    for &(target, migrate_fn) in MIGRATIONS {
        if target <= from_version {
            continue;
        }
        if target > CURRENT_CONFIG_VERSION {
            break;
        }
        migrate_fn(&mut doc, &mut warnings).map_err(|reason| MigrationError::StepFailed {
            from: target - 1,
            to: target,
            reason,
        })?;
    }

    let result = MigrationResult {
        from_version,
        to_version: CURRENT_CONFIG_VERSION,
        warnings,
    };

    Ok((doc.to_string(), Some(result)))
}

// ---------------------------------------------------------------------------
// Migration steps
// ---------------------------------------------------------------------------

/// v0 → v1: insert `config_version = 1` at the top of the document.
fn migrate_v0_to_v1(doc: &mut DocumentMut, warnings: &mut Vec<String>) -> Result<(), String> {
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

    Ok(())
}

/// v1 → v2: add `[agent]`, `[tools]`, and `[os.sudo]` sections with defaults
/// if they are absent. Also bumps `config_version` to 2.
fn migrate_v1_to_v2(doc: &mut DocumentMut, warnings: &mut Vec<String>) -> Result<(), String> {
    let mut added = Vec::new();

    if doc.get("agent").is_none() {
        let mut agent = Table::new();
        agent.insert("id", value("orka-default"));
        agent.insert("display_name", value("Orka"));
        agent.insert("max_iterations", value(15i64));
        agent.insert("heartbeat_interval_secs", value(30i64));
        doc.insert("agent", Item::Table(agent));
        added.push("[agent]");
    }

    if doc.get("tools").is_none() {
        let mut tools = Table::new();
        let mut disabled = Array::new();
        disabled.push("echo");
        tools.insert("disabled", value(disabled));
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
        sudo.insert("enabled", value(false));
        let mut allowed = Array::new();
        allowed.push("systemctl restart");
        allowed.push("systemctl stop");
        allowed.push("systemctl start");
        sudo.insert("allowed_commands", value(allowed));
        sudo.insert("require_confirmation", value(true));
        sudo.insert("confirmation_timeout_secs", value(120i64));
        sudo.insert("sudo_path", value("/usr/bin/sudo"));
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

    Ok(())
}

/// v2 → v3: convert `os.claude_code.enabled` from bool to string tri-state.
///
/// In v2 the field was `bool`; in v3 it is `"auto" | "true" | "false"`.
/// If the field is already a string or absent, this is a no-op (beyond the
/// version bump).
fn migrate_v2_to_v3(doc: &mut DocumentMut, warnings: &mut Vec<String>) -> Result<(), String> {
    if let Some(os_item) = doc.get_mut("os")
        && let Some(os_table) = os_item.as_table_mut()
        && let Some(cc_item) = os_table.get_mut("claude_code")
        && let Some(cc_table) = cc_item.as_table_mut()
        && let Some(enabled_item) = cc_table.get("enabled")
        && let Some(b) = enabled_item.as_bool()
    {
        let replacement = if b { "true" } else { "false" };
        cc_table.insert("enabled", value(replacement));
        warnings.push(format!(
            "Converted os.claude_code.enabled from boolean to \"{replacement}\". \
             Use \"auto\" to auto-detect claude CLI on PATH."
        ));
    }

    doc.insert("config_version", value(3i64));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_version_treated_as_v0_and_migrated() {
        let raw = r#"
[server]
host = "127.0.0.1"
port = 8080
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert_eq!(result.from_version, 0);
        assert_eq!(result.to_version, 3);
        assert!(!result.warnings.is_empty());

        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(read_version(&doc), 3);
    }

    #[test]
    fn current_version_is_noop() {
        let raw = r#"
config_version = 3

[server]
host = "127.0.0.1"
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        assert!(result.is_none());
        assert_eq!(migrated, raw);
    }

    #[test]
    fn future_version_rejected() {
        let raw = "config_version = 999\n";
        let err = migrate_if_needed(raw).unwrap_err();
        assert!(matches!(
            err,
            MigrationError::FutureVersion { found: 999, max: 3 }
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
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert_eq!(result.from_version, 1);
        assert_eq!(result.to_version, 3);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("[agent]"));
        assert!(result.warnings[0].contains("[tools]"));
        assert!(result.warnings[0].contains("[os.sudo]"));

        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(read_version(&doc), 3);

        // [agent] defaults
        let agent = doc["agent"].as_table().expect("[agent] missing");
        assert_eq!(agent["id"].as_str(), Some("orka-default"));
        assert_eq!(agent["display_name"].as_str(), Some("Orka"));
        assert_eq!(agent["max_iterations"].as_integer(), Some(15));
        assert_eq!(agent["heartbeat_interval_secs"].as_integer(), Some(30));

        // [tools] defaults
        let tools = doc["tools"].as_table().expect("[tools] missing");
        let disabled = tools["disabled"]
            .as_array()
            .expect("disabled array missing");
        assert_eq!(
            disabled.iter().next().and_then(|v| v.as_str()),
            Some("echo")
        );

        // [os.sudo] defaults
        let os = doc["os"].as_table().expect("[os] missing");
        let sudo = os["sudo"].as_table().expect("[os.sudo] missing");
        assert_eq!(sudo["enabled"].as_bool(), Some(false));
        assert_eq!(sudo["require_confirmation"].as_bool(), Some(true));
        assert_eq!(sudo["confirmation_timeout_secs"].as_integer(), Some(120));
        assert_eq!(sudo["sudo_path"].as_str(), Some("/usr/bin/sudo"));
    }

    #[test]
    fn v1_to_v2_preserves_existing_sections() {
        let raw = r#"config_version = 1

[agent]
id = "my-agent"
display_name = "MyBot"
max_iterations = 5
heartbeat_interval_secs = 60

[tools]
disabled = []

[os]
enabled = true

[os.sudo]
enabled = true
allowed_commands = ["apt-get install"]
require_confirmation = false
confirmation_timeout_secs = 60
sudo_path = "/usr/bin/sudo"
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert_eq!(result.from_version, 1);
        assert_eq!(result.to_version, 3);
        // No sections were added or converted, so no warnings
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );

        let doc: DocumentMut = migrated.parse().unwrap();
        // Existing values preserved
        assert_eq!(doc["agent"]["id"].as_str(), Some("my-agent"));
        assert_eq!(doc["agent"]["max_iterations"].as_integer(), Some(5));
        let os = doc["os"].as_table().unwrap();
        let sudo = os["sudo"].as_table().unwrap();
        assert_eq!(sudo["enabled"].as_bool(), Some(true));
    }

    #[test]
    fn v1_to_v2_no_os_section_skips_sudo() {
        let raw = "config_version = 1\n\n[server]\nhost = \"127.0.0.1\"\n";
        let (migrated, _) = migrate_if_needed(raw).unwrap();
        let doc: DocumentMut = migrated.parse().unwrap();
        assert!(doc.get("os").is_none(), "[os] should not be inserted");
    }

    #[test]
    fn comments_preserved_after_migration() {
        let raw = r#"# This is my config
[server]
host = "127.0.0.1" # inline comment
port = 8080
"#;
        let (migrated, _) = migrate_if_needed(raw).unwrap();
        assert!(migrated.contains("# This is my config"));
        assert!(migrated.contains("# inline comment"));
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let raw = "config_version = [invalid";
        let err = migrate_if_needed(raw).unwrap_err();
        assert!(matches!(err, MigrationError::Parse(_)));
    }

    #[test]
    fn v2_to_v3_converts_enabled_true() {
        let raw = r#"config_version = 2

[os.claude_code]
enabled = true
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert_eq!(result.from_version, 2);
        assert_eq!(result.to_version, 3);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("\"true\""));

        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(read_version(&doc), 3);
        assert_eq!(doc["os"]["claude_code"]["enabled"].as_str(), Some("true"));
    }

    #[test]
    fn v2_to_v3_converts_enabled_false() {
        let raw = r#"config_version = 2

[os.claude_code]
enabled = false
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert_eq!(result.from_version, 2);
        assert_eq!(result.to_version, 3);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("\"false\""));

        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(doc["os"]["claude_code"]["enabled"].as_str(), Some("false"));
    }

    #[test]
    fn v2_to_v3_no_claude_code_section_is_noop() {
        let raw = r#"config_version = 2

[server]
host = "127.0.0.1"
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert_eq!(result.from_version, 2);
        assert_eq!(result.to_version, 3);
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );

        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(read_version(&doc), 3);
    }

    #[test]
    fn v2_to_v3_string_enabled_unchanged() {
        // If someone already has enabled = "auto", it should be left as-is.
        let raw = r#"config_version = 2

[os.claude_code]
enabled = "auto"
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );

        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(doc["os"]["claude_code"]["enabled"].as_str(), Some("auto"));
    }

    #[test]
    fn v0_to_v3_chain() {
        let raw = r#"
[server]
host = "127.0.0.1"
port = 8080
"#;
        let (migrated, result) = migrate_if_needed(raw).unwrap();
        let result = result.expect("should have migration result");
        assert_eq!(result.from_version, 0);
        assert_eq!(result.to_version, 3);

        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(read_version(&doc), 3);
        // v1→v2 sections should be present
        assert!(doc.get("agent").is_some());
        assert!(doc.get("tools").is_some());
    }
}
