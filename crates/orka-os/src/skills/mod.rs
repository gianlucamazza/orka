pub mod system_info;
pub mod fs;
pub mod process;
pub mod shell;
pub mod network;
pub mod env;
pub mod package;

#[cfg(feature = "clipboard")]
pub mod clipboard;
#[cfg(feature = "desktop")]
pub mod notify;
#[cfg(feature = "desktop")]
pub mod desktop;
#[cfg(feature = "systemd")]
pub mod systemd;
