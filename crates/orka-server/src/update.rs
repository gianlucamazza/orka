//! Update checker: notifies at startup if a newer release exists on GitHub.

use tracing::warn;

const GITHUB_LATEST_URL: &str = "https://api.github.com/repos/gianlucamazza/orka/releases/latest";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Source-install command for the `orka-server` binary (`orka-server` package).
///
/// The crates.io name `orka` belongs to an unrelated project; never suggest
/// `cargo install orka`.
const CARGO_INSTALL_CMD: &str =
    "cargo install --locked --git https://github.com/gianlucamazza/orka orka-server";

/// Spawn a fire-and-forget task that checks for a newer release on GitHub
/// and logs a warning if one exists. Never blocks startup.
pub(crate) fn spawn_update_check() {
    tokio::spawn(async {
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent(format!("orka-server/{VERSION}"))
            .build()
        else {
            return;
        };
        let Ok(resp) = client
            .get(GITHUB_LATEST_URL)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
        else {
            return;
        };
        let Ok(json) = resp.json::<serde_json::Value>().await else {
            return;
        };
        let Some(tag) = json["tag_name"].as_str() else {
            return;
        };
        let latest = tag.trim_start_matches('v');
        if semver_gt(latest, VERSION) {
            let upgrade_hint = upgrade_hint_for_server();
            warn!(
                current = VERSION,
                latest, upgrade_hint, "A new version of orka-server is available."
            );
        }
    });
}

/// Returns an upgrade hint string based on the server binary's install
/// location.
fn upgrade_hint_for_server() -> &'static str {
    let is_docker = std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .is_ok_and(|c| c.contains("docker") || c.contains("containerd"));
    if is_docker {
        return "Pull the latest image and recreate the container.";
    }
    let Ok(exe) = std::env::current_exe() else {
        return "See https://github.com/gianlucamazza/orka/releases";
    };
    let s = exe.to_string_lossy();
    if s.starts_with("/usr/bin/") {
        "Run: yay -Syu orka-git"
    } else if s.starts_with("/usr/local/bin/") {
        "Run: orka update  (or re-run install.sh)"
    } else if s.contains("/.cargo/bin/") {
        "Run: cargo install --locked --git https://github.com/gianlucamazza/orka orka-server"
    } else {
        "See https://github.com/gianlucamazza/orka/releases"
    }
}

/// Returns true if `a` is a higher semver than `b` (major.minor.patch only).
pub(crate) fn semver_gt(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> (u64, u64, u64) {
        let mut it = v.split('.').filter_map(|p| p.parse::<u64>().ok());
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_gt_newer_major() {
        assert!(semver_gt("2.0.0", "1.0.0"));
    }

    #[test]
    fn semver_gt_older_returns_false() {
        assert!(!semver_gt("1.0.0", "2.0.0"));
    }

    #[test]
    fn semver_gt_equal_returns_false() {
        assert!(!semver_gt("1.2.3", "1.2.3"));
    }

    #[test]
    fn cargo_install_hint_uses_git_source_not_crates_io() {
        assert!(
            CARGO_INSTALL_CMD.contains("--git https://github.com/gianlucamazza/orka"),
            "server cargo upgrade must use this repo's git source: {CARGO_INSTALL_CMD}"
        );
        assert!(
            CARGO_INSTALL_CMD.ends_with(" orka-server"),
            "server cargo upgrade must install the orka-server package: {CARGO_INSTALL_CMD}"
        );
        assert!(
            !CARGO_INSTALL_CMD.contains("cargo install orka ")
                && !CARGO_INSTALL_CMD.ends_with("cargo install orka"),
            "must not suggest the unrelated crates.io crate `orka`: {CARGO_INSTALL_CMD}"
        );
    }
}
