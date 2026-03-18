//! Startup capability probing for OS skills.
//!
//! [`EnvironmentCapabilities`] probes available external commands at startup,
//! allowing skill registration to be conditioned on what actually works in the
//! current process environment (e.g. under `NoNewPrivileges`).

use std::time::Duration;

use orka_core::config::OsConfig;
use tracing::{debug, warn};

/// Result of probing a single external capability.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// Whether the capability is available and functional.
    pub available: bool,
    /// The specific method/command that was verified, if any.
    pub method: Option<String>,
    /// Non-fatal warnings discovered during probing.
    pub warnings: Vec<String>,
}

impl ProbeResult {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            method: None,
            warnings: vec![reason.to_string()],
        }
    }

    fn ok(method: impl Into<String>) -> Self {
        Self {
            available: true,
            method: Some(method.into()),
            warnings: Vec::new(),
        }
    }
}

/// Which method was probed and confirmed for checking package updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageUpdateMethod {
    /// `checkupdates` (pacman, safe, no root needed).
    CheckUpdates,
    /// `pacman -Qu` fallback (may show stale cache).
    PacmanQu,
    /// `apt list --upgradable`.
    AptListUpgradable,
    /// `dnf check-update`.
    DnfCheckUpdate,
}

/// Snapshot of runtime capabilities, computed once at startup.
#[derive(Debug)]
pub struct EnvironmentCapabilities {
    /// Whether `NoNewPrivileges` is set on the current process.
    pub no_new_privileges: bool,
    /// Package update check availability and chosen method.
    pub package_updates: ProbeResult,
    /// `systemctl` availability.
    pub systemctl: ProbeResult,
    /// `journalctl` availability.
    pub journalctl: ProbeResult,
    /// The confirmed package update method, if `package_updates.available`.
    pub update_method: Option<PackageUpdateMethod>,
    /// `claude` CLI availability.
    pub claude_code: ProbeResult,
}

impl EnvironmentCapabilities {
    /// Probe the runtime environment and return a capability snapshot.
    pub async fn probe(_config: &OsConfig) -> Self {
        let no_new_privileges = crate::has_no_new_privileges();

        if no_new_privileges {
            warn!(
                "NoNewPrivileges is active — checkupdates (fakeroot) will not work; \
                 will use pacman -Qu fallback if available"
            );
        }

        let (package_updates, update_method) = probe_package_updates(no_new_privileges).await;
        let systemctl = probe_systemctl().await;
        let journalctl = probe_journalctl().await;
        let claude_code = probe_claude_code().await;

        debug!(
            no_new_privileges,
            package_updates = package_updates.available,
            update_method = ?update_method,
            systemctl = systemctl.available,
            journalctl = journalctl.available,
            claude_code = claude_code.available,
            "environment capabilities probed"
        );

        Self {
            no_new_privileges,
            package_updates,
            systemctl,
            journalctl,
            update_method,
            claude_code,
        }
    }
}

async fn run_probe(cmd: &str, args: &[&str], timeout: Duration) -> bool {
    match tokio::time::timeout(
        timeout,
        tokio::process::Command::new(cmd).args(args).output(),
    )
    .await
    {
        Ok(Ok(out)) => {
            // For update-check commands, exit 0 (updates) or 2 (no updates, checkupdates)
            // or 1 (no updates, pacman -Qu) are all "working" results.
            // We accept any exit that isn't a crash/signal.
            out.status.code().is_some()
        }
        Ok(Err(e)) => {
            debug!(%e, cmd, "probe command spawn failed");
            false
        }
        Err(_) => {
            debug!(cmd, "probe command timed out");
            false
        }
    }
}

async fn probe_package_updates(
    no_new_privileges: bool,
) -> (ProbeResult, Option<PackageUpdateMethod>) {
    // Detect package manager
    if std::path::Path::new("/usr/bin/pacman").exists() {
        return probe_pacman(no_new_privileges).await;
    }
    if std::path::Path::new("/usr/bin/apt").exists() {
        let ok = run_probe("apt", &["list", "--upgradable"], Duration::from_secs(5)).await;
        if ok {
            return (
                ProbeResult::ok("apt list --upgradable"),
                Some(PackageUpdateMethod::AptListUpgradable),
            );
        }
        return (
            ProbeResult::unavailable("apt list --upgradable failed"),
            None,
        );
    }
    if std::path::Path::new("/usr/bin/dnf").exists() {
        // dnf check-update exits 0 (no updates) or 100 (updates); both are ok
        let ok = run_probe("dnf", &["check-update"], Duration::from_secs(5)).await;
        if ok {
            return (
                ProbeResult::ok("dnf check-update"),
                Some(PackageUpdateMethod::DnfCheckUpdate),
            );
        }
        return (ProbeResult::unavailable("dnf check-update failed"), None);
    }

    (
        ProbeResult::unavailable("no supported package manager found"),
        None,
    )
}

async fn probe_pacman(no_new_privileges: bool) -> (ProbeResult, Option<PackageUpdateMethod>) {
    // Try checkupdates first (preferred: uses a tmp db, no root needed)
    // but skip if NoNewPrivileges is set — fakeroot requires new privileges
    if !no_new_privileges && std::path::Path::new("/usr/bin/checkupdates").exists() {
        let ok = run_probe("checkupdates", &[], Duration::from_secs(5)).await;
        if ok {
            return (
                ProbeResult::ok("checkupdates"),
                Some(PackageUpdateMethod::CheckUpdates),
            );
        }
        warn!("checkupdates probe failed even though NoNewPrivileges is not set");
    }

    // Fallback: pacman -Qu (reads current sync db, may be stale)
    let ok = run_probe("pacman", &["-Qu"], Duration::from_secs(5)).await;
    if ok {
        let mut result = ProbeResult::ok("pacman -Qu");
        result
            .warnings
            .push("Using pacman -Qu fallback (stale cache possible)".to_string());
        return (result, Some(PackageUpdateMethod::PacmanQu));
    }

    (
        ProbeResult::unavailable("both checkupdates and pacman -Qu failed"),
        None,
    )
}

async fn probe_systemctl() -> ProbeResult {
    let ok = run_probe("systemctl", &["--version"], Duration::from_secs(2)).await;
    if ok {
        ProbeResult::ok("systemctl --version")
    } else {
        ProbeResult::unavailable("systemctl --version failed or not found")
    }
}

async fn probe_claude_code() -> ProbeResult {
    let ok = run_probe("claude", &["--version"], Duration::from_secs(2)).await;
    if ok {
        ProbeResult::ok("claude --version")
    } else {
        ProbeResult::unavailable("claude CLI not found or not functional")
    }
}

async fn probe_journalctl() -> ProbeResult {
    let ok = run_probe(
        "journalctl",
        &["-n", "1", "--no-pager"],
        Duration::from_secs(2),
    )
    .await;
    if ok {
        ProbeResult::ok("journalctl -n 1 --no-pager")
    } else {
        ProbeResult::unavailable("journalctl probe failed or not found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_result_ok_is_available() {
        let r = ProbeResult::ok("test-cmd");
        assert!(r.available);
        assert_eq!(r.method.as_deref(), Some("test-cmd"));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn probe_result_unavailable_not_available() {
        let r = ProbeResult::unavailable("no binary");
        assert!(!r.available);
        assert!(r.method.is_none());
        assert!(!r.warnings.is_empty());
    }

    #[tokio::test]
    async fn run_probe_missing_cmd_returns_false() {
        let ok = run_probe("__nonexistent_cmd_xyz__", &[], Duration::from_secs(1)).await;
        assert!(!ok);
    }
}
