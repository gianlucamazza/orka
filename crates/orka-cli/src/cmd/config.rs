use std::path::Path;

use orka_core::{
    config::OrkaConfig,
    migrate::{self, CURRENT_CONFIG_VERSION},
};

/// `orka config check` — validate config and show version + warnings.
#[allow(clippy::unused_async)]
pub async fn check(config_path: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path.map(Path::new);
    let resolved = OrkaConfig::resolve_path(path);

    if !resolved.exists() {
        return Err(format!("Config file not found: {}", resolved.display()).into());
    }

    let raw = std::fs::read_to_string(&resolved)?;
    let (_, result) = migrate::migrate_if_needed(&raw)?;
    let schema_issues = migrate::inspect_config_issues(&raw)?;

    match &result {
        Some(res) => {
            println!(
                "Config version: {} (current is {})",
                res.from_version, CURRENT_CONFIG_VERSION
            );
            if !res.warnings.is_empty() {
                println!("\nWarnings:");
                for w in &res.warnings {
                    println!("  - {w}");
                }
            }
            println!(
                "\nMigration available: v{} → v{}",
                res.from_version, res.to_version
            );
        }
        None => {
            println!("Config version: {CURRENT_CONFIG_VERSION} (up to date)");
        }
    }

    if !schema_issues.is_empty() {
        return Err(format!("Schema errors:\n  - {}", schema_issues.join("\n  - ")).into());
    }

    // Full deserialization + validation via OrkaConfig::load.
    let mut cfg = OrkaConfig::load(Some(&resolved))?;

    match cfg.validate() {
        Ok(()) => println!("\nValidation: OK"),
        Err(e) => return Err(format!("Validation error: {e}").into()),
    }

    Ok(())
}

/// `orka config migrate` — apply migrations and write the file (with backup).
#[allow(clippy::unused_async)]
pub async fn migrate_cmd(
    config_path: Option<&str>,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path.map(Path::new);
    let resolved = OrkaConfig::resolve_path(path);

    if !resolved.exists() {
        return Err(format!("Config file not found: {}", resolved.display()).into());
    }

    let raw = std::fs::read_to_string(&resolved)?;
    let (migrated_toml, result) = migrate::migrate_if_needed(&raw)?;
    let schema_issues = migrate::inspect_config_issues(&migrated_toml)?;

    match result {
        None => {
            println!(
                "Config is already at version {CURRENT_CONFIG_VERSION}. Nothing to do."
            );
        }
        Some(res) => {
            println!(
                "Migrating config: v{} → v{}",
                res.from_version, res.to_version
            );

            if !res.warnings.is_empty() {
                println!("\nWarnings:");
                for w in &res.warnings {
                    println!("  - {w}");
                }
            }

            if !schema_issues.is_empty() {
                return Err(format!("Schema errors:\n  - {}", schema_issues.join("\n  - ")).into());
            }

            if dry_run {
                println!("\n--- Diff (dry run) ---");
                print_diff(&raw, &migrated_toml);
                println!("--- End diff ---");
                println!("\nNo changes written (dry run).");
            } else {
                // Create backup.
                let backup_path = resolved.with_extension("toml.bak");
                std::fs::copy(&resolved, &backup_path)?;
                println!("\nBackup written to: {}", backup_path.display());

                // Write migrated config.
                std::fs::write(&resolved, &migrated_toml)?;
                println!("Config written to: {}", resolved.display());
            }
        }
    }

    Ok(())
}

/// Print a unified diff between two strings.
pub(crate) fn print_diff(old: &str, new: &str) {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        print!("{sign}{change}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // print_diff writes to stdout; we just verify it doesn't panic.
    #[test]
    fn print_diff_identical_input_no_panic() {
        print_diff("foo = 1\n", "foo = 1\n");
    }

    #[test]
    fn print_diff_different_input_no_panic() {
        print_diff("foo = 1\nbar = 2\n", "foo = 1\nbar = 3\n");
    }

    #[test]
    fn print_diff_empty_strings_no_panic() {
        print_diff("", "");
    }
}
