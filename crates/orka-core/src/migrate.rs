//! Config versioning and migration engine.
//!
//! Each config version is an integer. Migrations run sequentially (v0→v1→v2→…)
//! using `toml_edit` to preserve comments and formatting. The server migrates
//! in-memory at boot; only the CLI writes the file.

use toml_edit::DocumentMut;

/// The config version that this build of Orka expects.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

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
const MIGRATIONS: &[(u32, MigrateFn)] = &[(1, migrate_v0_to_v1)];

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
        assert_eq!(result.to_version, 1);
        assert!(!result.warnings.is_empty());

        // The migrated TOML should contain config_version = 1
        let doc: DocumentMut = migrated.parse().unwrap();
        assert_eq!(read_version(&doc), 1);
    }

    #[test]
    fn current_version_is_noop() {
        let raw = r#"
config_version = 1

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
            MigrationError::FutureVersion { found: 999, max: 1 }
        ));
        let msg = err.to_string();
        assert!(msg.contains("999"));
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
}
