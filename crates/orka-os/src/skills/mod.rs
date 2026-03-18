/// Claude Code delegation skill — runs `claude --print` as a subprocess.
pub mod claude_code;
/// Environment variable inspection skills.
pub mod env;
/// Filesystem skills: read, write, list, search, watch.
pub mod fs;
/// Network information and connectivity skills.
pub mod network;
/// Package manager skills: search, info, list, updates, install.
pub mod package;
/// Process listing, info, and signal skills.
pub mod process;
/// Shell command execution skill.
pub mod shell;
/// System information skill.
pub mod system_info;

/// Clipboard read/write skills (requires `clipboard` feature).
#[cfg(feature = "clipboard")]
pub mod clipboard;
/// Desktop open and screenshot skills (requires `desktop` feature).
#[cfg(feature = "desktop")]
pub mod desktop;
/// Desktop notification skill (requires `desktop` feature).
#[cfg(feature = "desktop")]
pub mod notify;
/// systemd service control and journal skills (requires `systemd` feature).
#[cfg(feature = "systemd")]
pub mod systemd;
