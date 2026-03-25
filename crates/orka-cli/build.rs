//! Build script for orka-cli: embeds git SHA and build date.

fn main() {
    // Trigger rebuild when git state changes.
    // Use CARGO_MANIFEST_DIR to resolve paths relative to workspace root (2 levels
    // up from crates/orka-cli/).
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let git_head = manifest_dir.join("../../.git/HEAD");
    let git_refs = manifest_dir.join("../../.git/refs/");
    println!("cargo:rerun-if-changed={}", git_head.display());
    println!("cargo:rerun-if-changed={}", git_refs.display());

    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".to_string(), |s| s.trim().to_string());

    let date = std::process::Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    println!("cargo:rustc-env=ORKA_GIT_SHA={sha}");
    println!("cargo:rustc-env=ORKA_BUILD_DATE={date}");
}
