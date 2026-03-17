use orka_core::config::OrkaConfig;
use std::path::Path;
use std::process::Stdio;

pub async fn check(config_path: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path.map(Path::new);
    let config = OrkaConfig::load(path)?;

    if !config.os.sudo.enabled {
        println!("sudo is disabled in configuration");
        return Ok(());
    }

    println!("sudo path: {}", config.os.sudo.sudo_path);
    println!(
        "require confirmation: {}",
        config.os.sudo.require_confirmation
    );
    println!(
        "confirmation timeout: {}s",
        config.os.sudo.confirmation_timeout_secs
    );
    println!();

    if config.os.sudo.allowed_commands.is_empty() {
        println!("no allowed commands configured in [os.sudo]");
        return Ok(());
    }

    println!(
        "checking {} allowed command(s) against sudoers...\n",
        config.os.sudo.allowed_commands.len()
    );

    let sudo_path = &config.os.sudo.sudo_path;
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
                println!("  OK  {}", cmd);
            }
            Ok(_) => {
                println!("  FAIL  {} (not in sudoers NOPASSWD or sudo -n failed)", cmd);
                all_ok = false;
            }
            Err(e) => {
                println!("  ERR   {} ({})", cmd, e);
                all_ok = false;
            }
        }
    }

    println!();
    if all_ok {
        println!("all commands have NOPASSWD access");
    } else {
        println!("some commands lack NOPASSWD access");
        println!(
            "hint: create /etc/sudoers.d/orka with appropriate NOPASSWD entries"
        );
    }

    Ok(())
}
