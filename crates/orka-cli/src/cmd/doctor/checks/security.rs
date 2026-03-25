use std::process::Stdio;

use async_trait::async_trait;

use crate::cmd::doctor::{
    CheckContext, DoctorCheck,
    types::{Category, CheckId, CheckMeta, CheckOutcome, FixAction, Severity},
};

pub struct SecNoInlineKeys;
pub struct SecFilePermissions;
pub struct SecWorkspaceDirs;
pub struct SecSudoConfig;
pub struct SecNoNewPrivileges;

#[async_trait]
impl DoctorCheck for SecNoInlineKeys {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("SEC-001"),
            category: Category::Security,
            severity: Severity::Warning,
            name: "No inline API keys",
            description: "API keys should not be stored inline in orka.toml.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        // This duplicates CFG-005 logic intentionally: SEC checks are about security
        // posture, CFG checks are about config validity. Having both makes
        // filtering by category useful.
        let raw = match &ctx.config_raw {
            Some(r) => r,
            None => return CheckOutcome::skip("config file not readable"),
        };

        // Scan for lines with api_key = "..." that contain long strings (likely real
        // keys)
        let mut suspicious = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with("api_key") && trimmed.contains('=') && trimmed.contains('"') {
                // Extract the value part
                if let Some(val) = extract_string_value(trimmed)
                    && val.len() > 8
                    && !val.contains('/')
                    && !val.contains('.')
                {
                    suspicious.push(format!("line {}: api_key = \"{}...\"", i + 1, &val[..4]));
                }
            }
        }

        if suspicious.is_empty() {
            CheckOutcome::pass("no inline API keys detected")
        } else {
            CheckOutcome::fail(format!("{} potential inline key(s)", suspicious.len()))
                .with_detail(suspicious.join(", "))
                .with_hint(
                    "Move secrets to environment variables (api_key_env) or the secret store \
                     (api_key_secret) to avoid accidental leaks.",
                )
        }
    }

    fn explain(&self) -> &'static str {
        "Inline API keys in orka.toml risk being committed to version control or exposed \
         in backups. Use api_key_env = \"ENV_VAR\" to reference an environment variable, \
         or api_key_secret = \"path/in/store\" to use the Redis-backed secret store. \
         Run `orka secret set <path>` to store secrets securely."
    }
}

fn extract_string_value(line: &str) -> Option<String> {
    let eq_pos = line.find('=')?;
    let after_eq = line[eq_pos + 1..].trim();
    if after_eq.starts_with('"') && after_eq.ends_with('"') && after_eq.len() > 2 {
        Some(after_eq[1..after_eq.len() - 1].to_string())
    } else {
        None
    }
}

#[async_trait]
impl DoctorCheck for SecFilePermissions {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("SEC-002"),
            category: Category::Security,
            severity: Severity::Warning,
            name: "Config file permissions",
            description: "orka.toml should not be readable by group or others (mode 600).",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        if !ctx.config_path.exists() {
            return CheckOutcome::skip("config file does not exist");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            match std::fs::metadata(&ctx.config_path) {
                Ok(meta) => {
                    let mode = meta.permissions().mode();
                    if mode & 0o077 == 0 {
                        CheckOutcome::pass(format!("mode {:o}", mode & 0o777))
                    } else {
                        let path = ctx.config_path.clone();
                        CheckOutcome::fail(format!(
                            "mode {:o} — group/others can read",
                            mode & 0o777
                        ))
                        .with_hint("Run `chmod 600 orka.toml` to restrict permissions.")
                        .with_fix(FixAction {
                            description: "chmod 600 orka.toml".to_string(),
                            apply: Box::new(move || {
                                std::fs::set_permissions(
                                    &path,
                                    std::fs::Permissions::from_mode(0o600),
                                )?;
                                Ok("permissions set to 600".to_string())
                            }),
                        })
                    }
                }
                Err(e) => CheckOutcome::fail(format!("cannot stat config file: {e}")),
            }
        }

        #[cfg(not(unix))]
        CheckOutcome::skip("permission check not supported on this OS")
    }
}

#[async_trait]
impl DoctorCheck for SecWorkspaceDirs {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("SEC-003"),
            category: Category::Security,
            severity: Severity::Error,
            name: "Workspace directories exist",
            description: "All configured workspace directories must exist and be writable.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        let mut missing = Vec::new();
        let mut not_writable = Vec::new();

        // Check primary workspace dir
        let primary = std::path::Path::new(&config.workspace_dir);
        if !primary.exists() {
            missing.push(config.workspace_dir.clone());
        } else if is_not_writable(primary) {
            not_writable.push(config.workspace_dir.clone());
        }

        // Check additional workspaces
        for ws in &config.workspaces {
            let p = std::path::Path::new(&ws.dir);
            if !p.exists() {
                missing.push(ws.dir.clone());
            } else if is_not_writable(p) {
                not_writable.push(ws.dir.clone());
            }
        }

        if missing.is_empty() && not_writable.is_empty() {
            let total = 1 + config.workspaces.len();
            CheckOutcome::pass(format!("{total} workspace dir(s) OK"))
        } else {
            let mut msg_parts = Vec::new();
            if !missing.is_empty() {
                msg_parts.push(format!("missing: {}", missing.join(", ")));
            }
            if !not_writable.is_empty() {
                msg_parts.push(format!("not writable: {}", not_writable.join(", ")));
            }
            CheckOutcome::fail(msg_parts.join("; ")).with_hint(
                "Create missing directories and ensure they are writable by the orka process.",
            )
        }
    }
}

fn is_not_writable(path: &std::path::Path) -> bool {
    // Try to create a temp file to test writability
    tempfile::tempfile_in(path).is_err()
}

#[async_trait]
impl DoctorCheck for SecSudoConfig {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("SEC-004"),
            category: Category::Security,
            severity: Severity::Warning,
            name: "Sudo configuration valid",
            description: "Sudoers and systemd drop-in must be present when os.sudo.allowed = true.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        if !config.os.sudo.allowed {
            return CheckOutcome::skip("os.sudo.allowed = false");
        }

        const DROPIN: &str = "/etc/systemd/system/orka-server.service.d/sudo.conf";
        const SUDOERS: &str = "/etc/sudoers.d/orka";

        let dropin_ok = path_exists_elevated(DROPIN).await;
        let sudoers_ok = path_exists_elevated(SUDOERS).await;

        if dropin_ok && sudoers_ok {
            CheckOutcome::pass("systemd drop-in and sudoers file found")
        } else {
            let mut missing = Vec::new();
            if !dropin_ok {
                missing.push(DROPIN);
            }
            if !sudoers_ok {
                missing.push(SUDOERS);
            }
            CheckOutcome::fail(format!("missing: {}", missing.join(", "))).with_hint(
                "Run scripts/install.sh to install the required sudo configuration files.",
            )
        }
    }

    fn explain(&self) -> &'static str {
        "When os.sudo.allowed = true, Orka can run privileged commands. \
         This requires two files: a systemd drop-in at \
         /etc/systemd/system/orka-server.service.d/sudo.conf (to disable NoNewPrivileges) \
         and a sudoers file at /etc/sudoers.d/orka (to grant NOPASSWD for allowed commands). \
         Run `orka sudo` for the full diagnostic."
    }
}

#[async_trait]
impl DoctorCheck for SecNoNewPrivileges {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("SEC-005"),
            category: Category::Security,
            severity: Severity::Warning,
            name: "NoNewPrivileges check",
            description: "NoNewPrivileges=yes blocks sudo; must be disabled when os.sudo.allowed.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        if !config.os.sudo.allowed {
            return CheckOutcome::skip("os.sudo.allowed = false");
        }

        match tokio::process::Command::new("systemctl")
            .args(["show", "-p", "NoNewPrivileges", "orka-server.service"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("NoNewPrivileges=yes") {
                    CheckOutcome::fail("NoNewPrivileges=yes — sudo will not work").with_hint(
                        "Install the systemd drop-in with scripts/install.sh to disable \
                             NoNewPrivileges for orka-server.service.",
                    )
                } else if stdout.contains("NoNewPrivileges=no") {
                    CheckOutcome::pass("NoNewPrivileges=no (sudo allowed)")
                } else {
                    CheckOutcome::skip("service not loaded or systemctl unavailable")
                }
            }
            Err(_) => CheckOutcome::skip("systemctl not available"),
        }
    }
}

async fn path_exists_elevated(path: &str) -> bool {
    if std::path::Path::new(path).exists() {
        return true;
    }
    // Try elevated check for paths we can't stat directly
    tokio::process::Command::new("sudo")
        .args(["-n", "test", "-f", path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}
