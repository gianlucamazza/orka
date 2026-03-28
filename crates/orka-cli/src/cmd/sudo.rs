use std::{path::Path, process::Stdio};

use orka_config::OrkaConfig;

const DROPIN_PATH: &str = "/etc/systemd/system/orka-server.service.d/sudo.conf";
const SUDOERS_PATH: &str = "/etc/sudoers.d/orka";

/// Check if a path exists, falling back to `sudo -n test -f` when the current
/// user lacks permission to stat the parent directory (e.g. `/etc/sudoers.d/`).
async fn path_exists_elevated(sudo_path: &str, path: &str) -> bool {
    if Path::new(path).exists() {
        return true;
    }
    tokio::process::Command::new(sudo_path)
        .args(["-n", "test", "-f", path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

#[allow(clippy::too_many_lines)]
pub async fn check(config_path: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path.map(Path::new);
    let config = OrkaConfig::load(path)?;

    if !config.os.sudo.allowed {
        println!("sudo is not allowed in configuration");
        return Ok(());
    }

    println!("sudo allowed: {}", config.os.sudo.allowed);
    println!("password required: {}", config.os.sudo.password_required);
    println!(
        "allowed commands: {}",
        config.os.sudo.allowed_commands.join(", ")
    );
    println!();

    let sudo_path = "sudo"; // Use default sudo path

    // --- Environment checks ---
    let mut env_ok = true;

    // Check NoNewPrivileges via systemctl
    print!("  NoNewPrivileges ... ");
    match tokio::process::Command::new("systemctl")
        .args(["show", "-p", "NoNewPrivileges", "orka-server.service"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("NoNewPrivileges=yes") {
                println!("FAIL (active — sudo will not work)");
                println!("        hint: install the systemd drop-in with scripts/install.sh");
                env_ok = false;
            } else if stdout.contains("NoNewPrivileges=no") {
                println!("OK (disabled)");
            } else {
                println!("SKIP (could not determine, service may not be loaded)");
            }
        }
        Err(_) => println!("SKIP (systemctl not available)"),
    }

    // Check drop-in exists
    print!("  systemd drop-in ... ");
    if path_exists_elevated(sudo_path, DROPIN_PATH).await {
        println!("OK ({DROPIN_PATH})");
    } else {
        println!("MISSING ({DROPIN_PATH})");
        println!("        hint: run scripts/install.sh to create it");
        env_ok = false;
    }

    // Check sudoers file exists
    print!("  sudoers file    ... ");
    if path_exists_elevated(sudo_path, SUDOERS_PATH).await {
        println!("OK ({SUDOERS_PATH})");
    } else {
        println!("MISSING ({SUDOERS_PATH})");
        println!("        hint: run scripts/install.sh to create it");
        env_ok = false;
    }

    println!();

    if !env_ok {
        println!("environment has issues — fix them before testing commands\n");
    }

    // --- Command checks ---
    if config.os.sudo.allowed_commands.is_empty() {
        println!("no allowed commands configured in [os.sudo]");
        return Ok(());
    }

    println!(
        "checking {} allowed command(s) against sudoers...\n",
        config.os.sudo.allowed_commands.len()
    );

    let mut all_ok = true;

    for cmd in &config.os.sudo.allowed_commands {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        // Use `sudo -n -l <command>` to check if NOPASSWD is configured
        let result = tokio::process::Command::new(sudo_path)
            .args(["-n", "-l"])
            .arg(parts[0])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                println!("  OK    {cmd}");
            }
            Ok(_) => {
                println!("  FAIL  {cmd} (not in sudoers NOPASSWD or sudo -n failed)");
                all_ok = false;
            }
            Err(e) => {
                println!("  ERR   {cmd} ({e})");
                all_ok = false;
            }
        }
    }

    println!();
    if all_ok && env_ok {
        println!("all checks passed");
    } else if all_ok {
        println!("all commands checked, but environment issues remain");
    } else {
        println!("some commands have issues");
        println!("hint: create /etc/sudoers.d/orka with appropriate NOPASSWD entries");
    }

    Ok(())
}
