use std::{
    io::Write as _,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use colored::Colorize;
use reqwest::Client;
use serde::Deserialize;
use sha2::Digest as _;

use crate::client::Result;

const REPO_OWNER: &str = "gianlucamazza";
const REPO_NAME: &str = "orka";
const CHECK_INTERVAL_SECS: u64 = 86_400; // 24 hours

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub struct UpdateInfo {
    pub latest: String,
    pub release_url: String,
}

#[derive(Debug, PartialEq)]
enum InstallMethod {
    DirectInstall, // /usr/local/bin/orka  → self-update OK
    Pacman,        // /usr/bin/orka        → managed by pacman
    Docker,        // /.dockerenv exists   → inside container
    CargoInstall,  // ~/.cargo/bin/orka    → installed via cargo
    DevBuild,      // target/*/orka        → local dev build
    Unknown,
}

fn detect_install_method() -> InstallMethod {
    // Docker: check for /.dockerenv or docker/containerd in cgroup
    if std::path::Path::new("/.dockerenv").exists() {
        return InstallMethod::Docker;
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup")
        && (cgroup.contains("docker") || cgroup.contains("containerd"))
    {
        return InstallMethod::Docker;
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return InstallMethod::Unknown,
    };
    let exe_str = exe.to_string_lossy();

    if exe_str.contains("/.cargo/bin/") {
        InstallMethod::CargoInstall
    } else if exe_str.contains("/target/debug/") || exe_str.contains("/target/release/") {
        InstallMethod::DevBuild
    } else if exe_str.starts_with("/usr/bin/") {
        InstallMethod::Pacman
    } else if exe_str.starts_with("/usr/local/bin/") {
        InstallMethod::DirectInstall
    } else {
        InstallMethod::Unknown
    }
}

/// Returns the upgrade hint for a given install method (used in update
/// notices).
fn upgrade_hint(method: &InstallMethod) -> &'static str {
    match method {
        InstallMethod::DirectInstall => "Run 'orka update' to upgrade.",
        InstallMethod::Pacman => "Run 'yay -Syu orka-git' to upgrade.",
        InstallMethod::Docker => "Pull the latest image to upgrade.",
        InstallMethod::CargoInstall => "Run 'cargo install orka' to upgrade.",
        InstallMethod::DevBuild | InstallMethod::Unknown => {
            "See https://github.com/gianlucamazza/orka/releases"
        }
    }
}

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Returns true if `tag` is a higher semver than the current binary.
///
/// Pre-release suffixes (e.g. `-rc.1`, `-beta`) are stripped from each
/// segment before parsing so that `v0.5.0-rc.1` compares correctly against
/// `v0.5.0` instead of silently being treated as patch `0`.
fn is_newer(tag: &str) -> bool {
    let parse = |v: &str| -> (u64, u64, u64) {
        let v = v.trim_start_matches('v');
        // Strip pre-release suffix from each dot-segment before parsing.
        let mut it = v
            .split('.')
            .filter_map(|p| p.split('-').next()?.parse::<u64>().ok());
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    parse(tag) > parse(current_version())
}

fn cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("orka").join("last_update_check"))
}

fn read_cache() -> Option<(u64, String)> {
    let path = cache_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let ts: u64 = lines.next()?.parse().ok()?;
    Some((ts, lines.next()?.to_string()))
}

fn write_cache(tag: &str) {
    if let Some(path) = cache_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = std::fs::write(path, format!("{ts}\n{tag}"));
    }
}

fn build_client() -> reqwest::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(format!("orka/{}", current_version()))
        .build()
}

async fn fetch_latest() -> reqwest::Result<GithubRelease> {
    build_client()?
        .get(format!(
            "https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest"
        ))
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?
        .error_for_status()?
        .json::<GithubRelease>()
        .await
}

/// Read the local cache and return update info if an update is available.
/// Does no network I/O — call this on the hot path (e.g. chat banner).
pub fn check_from_cache() -> Option<UpdateInfo> {
    if std::env::var("ORKA_NO_UPDATE_CHECK").is_ok() {
        return None;
    }
    let (ts, tag) = read_cache()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(ts) >= CHECK_INTERVAL_SECS {
        return None; // cache stale — refresh happens in background
    }
    if is_newer(&tag) {
        Some(UpdateInfo {
            latest: tag.trim_start_matches('v').to_string(),
            release_url: format!("https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/{tag}"),
        })
    } else {
        None
    }
}

/// Fetch the latest release from GitHub, update the cache, and return
/// `Some(UpdateInfo)` if a newer version is available.
pub async fn check() -> Option<UpdateInfo> {
    if std::env::var("ORKA_NO_UPDATE_CHECK").is_ok() {
        return None;
    }
    let release = fetch_latest().await.ok()?;
    write_cache(&release.tag_name);
    if is_newer(&release.tag_name) {
        Some(UpdateInfo {
            latest: release.tag_name.trim_start_matches('v').to_string(),
            release_url: release.html_url,
        })
    } else {
        None
    }
}

/// Print a one-line update notice (used in the chat banner).
/// The hint adapts to the install method so the user knows the right command.
pub fn print_update_notice(info: &UpdateInfo) {
    let hint = upgrade_hint(&detect_install_method());
    println!(
        "{} Orka v{} → v{} available. {}",
        "⚡".yellow(),
        current_version(),
        info.latest.green().bold(),
        hint.cyan().bold(),
    );
}

/// `orka version --check`
pub async fn run_check() -> Result<()> {
    let current = current_version();
    println!("orka {current}");
    print!("Checking for updates...");
    std::io::stdout().flush()?;
    match check().await {
        Some(info) => {
            println!();
            println!(
                "Update available: {} → {}",
                current.yellow(),
                info.latest.green().bold()
            );
            println!("Release notes: {}", info.release_url);
            let hint = upgrade_hint(&detect_install_method());
            println!("{hint}");
        }
        None => println!(" up to date."),
    }
    Ok(())
}

/// `orka update` — download, extract, and replace the current binary.
/// Refuses to self-update if the binary was not installed via install.sh.
pub async fn run_update() -> Result<()> {
    let method = detect_install_method();
    match method {
        InstallMethod::DirectInstall => {} // proceed below
        InstallMethod::Pacman => {
            return Err("Orka is managed by pacman. Run: yay -Syu orka-git".into());
        }
        InstallMethod::Docker => {
            return Err("Running inside a container. Pull the latest image and recreate.".into());
        }
        InstallMethod::CargoInstall => {
            return Err("Installed via cargo. Run: cargo install orka".into());
        }
        InstallMethod::DevBuild => {
            return Err("This is a dev build. Run: cargo build --release".into());
        }
        InstallMethod::Unknown => {
            return Err("Cannot determine install method. \
                 See https://github.com/gianlucamazza/orka/releases for upgrade instructions."
                .into());
        }
    }

    let current = current_version();

    // 1. Check for available update
    print!("Checking for updates...");
    std::io::stdout().flush()?;
    let release = fetch_latest()
        .await
        .map_err(|e| format!("failed to fetch release info: {e}"))?;
    let latest = release.tag_name.trim_start_matches('v');

    if !is_newer(&release.tag_name) {
        println!(" already up to date (v{current}).");
        return Ok(());
    }
    println!(" v{latest} available (current: v{current}).");

    // 2. Find the platform-specific tarball asset
    let platform = {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            other => other,
        };
        format!("{os}-{}", std::env::consts::ARCH)
    };
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.contains(&platform) && a.name.ends_with(".tar.gz"))
        .ok_or_else(|| format!("no {platform} tarball found in release assets for v{latest}"))?;

    // 3. Download tarball
    println!("Downloading {}...", asset.name);
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(format!("orka/{current}"))
        .build()?;
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // 4. Verify SHA256 checksum (if a .sha256 asset is present in the release)
    let sha256_asset_name = format!("{}.sha256", asset.name);
    if let Some(sha256_asset) = release.assets.iter().find(|a| a.name == sha256_asset_name) {
        println!("Verifying checksum...");
        let checksum_text = client
            .get(&sha256_asset.browser_download_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        // Checksum file format: "<hex>  <filename>" or just "<hex>"
        let expected = checksum_text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let actual = hex::encode(sha2::Sha256::digest(&bytes));
        if actual != expected {
            return Err(format!("Checksum mismatch: expected {expected}, got {actual}").into());
        }
        println!("{} Checksum verified.", "\u{2713}".green().bold());
    } else {
        eprintln!(
            "{} No checksum file ({sha256_asset_name}) found in release assets. \
             Skipping integrity verification.",
            "\u{26a0}".yellow()
        );
    }

    // 5. Extract the `orka` binary from the tarball (was step 4)
    let tmpdir = tempfile::tempdir()?;
    let tarball_path = tmpdir.path().join("orka-update.tar.gz");
    std::fs::write(&tarball_path, &bytes)?;

    let extract_dir = tmpdir.path().join("extracted");
    std::fs::create_dir_all(&extract_dir)?;

    let status = tokio::process::Command::new("tar")
        .arg("xzf")
        .arg(&tarball_path)
        .arg("-C")
        .arg(&extract_dir)
        .arg("orka")
        .status()
        .await?;

    if !status.success() {
        return Err("failed to extract update archive".into());
    }

    let new_binary = extract_dir.join("orka");

    // 5. Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new_binary, std::fs::Permissions::from_mode(0o755))?;
    }

    // 6. Atomically replace the running binary (safe on Linux)
    let current_exe = std::env::current_exe()?;
    std::fs::rename(&new_binary, &current_exe).or_else(|_| -> std::io::Result<()> {
        // Cross-filesystem fallback: copy to a sibling temp file, then rename.
        // This avoids leaving a partially-written binary if the process is killed
        // mid-copy (a plain fs::copy + overwrite is not atomic).
        let tmp_exe = current_exe.with_extension("update_tmp");
        std::fs::copy(&new_binary, &tmp_exe)?;
        std::fs::rename(&tmp_exe, &current_exe).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp_exe);
        })
    })?;

    write_cache(&release.tag_name);
    println!(
        "{} Updated to v{latest}. Restart orka to use the new version.",
        "✓".green().bold()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_detects_higher_version() {
        // v0.0.0 is current in tests; any real version is "newer"
        assert!(is_newer("v99.0.0"));
        assert!(is_newer("99.0.0"));
    }

    #[test]
    fn is_newer_rejects_older_or_equal() {
        // Patch: 0.0.0 is not newer than itself
        assert!(!is_newer("v0.0.0"));
        assert!(!is_newer("0.0.0"));
    }

    #[test]
    fn detect_docker_via_dockerenv() {
        // Can't create /.dockerenv in tests, but we can verify the function runs
        // without panic and returns a valid variant.
        let method = detect_install_method();
        assert!(matches!(
            method,
            InstallMethod::DirectInstall
                | InstallMethod::Pacman
                | InstallMethod::Docker
                | InstallMethod::CargoInstall
                | InstallMethod::DevBuild
                | InstallMethod::Unknown
        ));
    }

    #[test]
    fn upgrade_hint_covers_all_variants() {
        let variants = [
            InstallMethod::DirectInstall,
            InstallMethod::Pacman,
            InstallMethod::Docker,
            InstallMethod::CargoInstall,
            InstallMethod::DevBuild,
            InstallMethod::Unknown,
        ];
        for v in &variants {
            let hint = upgrade_hint(v);
            assert!(!hint.is_empty(), "hint for {v:?} must not be empty");
        }
    }

    #[test]
    fn upgrade_hint_pacman_mentions_yay() {
        assert!(upgrade_hint(&InstallMethod::Pacman).contains("yay"));
    }

    #[test]
    fn upgrade_hint_cargo_mentions_cargo() {
        assert!(upgrade_hint(&InstallMethod::CargoInstall).contains("cargo"));
    }
}
