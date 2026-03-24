use std::{path::Path, sync::Arc};

use orka_core::{config::PluginConfig, traits::Skill};
use orka_wasm::{PluginCapabilities, WasmEngine};
use tracing::{info, warn};

/// Scan `dir` for `.wasm` files and load each as a WASM Component plugin skill.
///
/// Per-plugin configuration is read from `plugin_config.plugins`. Global
/// capabilities from `plugin_config.capabilities` are used as defaults.
/// Missing or empty directories are silently skipped. Files that fail to load
/// are logged as warnings and skipped rather than aborting the whole scan.
pub fn load_plugins(
    dir: &Path,
    engine: &WasmEngine,
    plugin_config: &PluginConfig,
) -> orka_core::Result<Vec<Arc<dyn Skill>>> {
    let mut plugins = Vec::new();

    if !dir.exists() {
        info!(?dir, "plugin directory does not exist, skipping");
        return Ok(plugins);
    }

    let entries = std::fs::read_dir(dir)
        .map_err(|e| orka_core::Error::Skill(format!("failed to read plugin dir: {e}")))?;

    for entry in entries {
        let entry = entry.map_err(|e| orka_core::Error::Skill(e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wasm") {
            let plugin_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            // Build capabilities: start with global defaults, override with per-plugin
            // config
            let caps = PluginCapabilities {
                env: vec![], // TODO: support env var allowlist from config
                fs: if plugin_config.capabilities.filesystem {
                    vec![".".to_string()] // Allow current dir
                } else {
                    vec![]
                },
                network: plugin_config.capabilities.network,
            };

            // Override with per-plugin config if present
            // TODO: support per-plugin capability overrides via instance_config.config
            if let Some(instance_config) = plugin_config.plugins.get(&plugin_name)
                && !instance_config.enabled
            {
                info!(name = %plugin_name, "plugin disabled in config, skipping");
                continue;
            }

            match super::wasm_plugin::WasmPluginSkill::load(&path, engine, caps) {
                Ok(skill) => {
                    info!(name = skill.name(), path = ?path, "loaded WASM plugin");
                    plugins.push(Arc::new(skill) as Arc<dyn Skill>);
                }
                Err(e) => {
                    warn!(?path, %e, "failed to load WASM plugin");
                }
            }
        }
    }

    Ok(plugins)
}
