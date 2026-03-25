use std::{path::Path, sync::Arc};

use orka_core::{
    config::{PluginCapabilities as ConfigCaps, PluginConfig, PluginInstanceConfig},
    traits::Skill,
};
use orka_wasm::{PluginCapabilities, WasmEngine};
use tracing::{info, warn};

/// Scan `dir` for `.wasm` files and load each as a WASM Component plugin skill.
///
/// Per-plugin configuration is read from `plugin_config.plugins`. Global
/// capabilities from `plugin_config.capabilities` are used as defaults;
/// per-plugin `capabilities` tables override individual fields when present.
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

            let instance_config = plugin_config.plugins.get(&plugin_name);

            if let Some(ic) = instance_config
                && !ic.enabled
            {
                info!(name = %plugin_name, "plugin disabled in config, skipping");
                continue;
            }

            let caps = resolve_capabilities(&plugin_config.capabilities, instance_config);

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

/// Merge global capability defaults with optional per-plugin overrides.
///
/// Each field in the runtime [`PluginCapabilities`] is taken from the
/// per-plugin `capabilities` table when present, otherwise the global default
/// applies.
fn resolve_capabilities(
    global: &ConfigCaps,
    instance: Option<&PluginInstanceConfig>,
) -> PluginCapabilities {
    let overrides = instance.and_then(|i| i.capabilities.as_ref());
    PluginCapabilities {
        env: overrides
            .map(|o| o.env.clone())
            .unwrap_or_else(|| global.env.clone()),
        fs: overrides
            .map(|o| o.filesystem.clone())
            .unwrap_or_else(|| global.filesystem.clone()),
        network: overrides.map(|o| o.network).unwrap_or(global.network),
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::PluginCapabilities as ConfigCaps;

    use super::*;

    // `ConfigCaps` and `PluginInstanceConfig` are #[non_exhaustive], so we
    // cannot use struct literals outside their defining crate.  Use Default +
    // field assignment instead.

    fn config_caps(fs: &[&str], network: bool, env: &[&str]) -> ConfigCaps {
        let mut c = ConfigCaps::default();
        c.filesystem = fs.iter().map(|s| s.to_string()).collect();
        c.network = network;
        c.env = env.iter().map(|s| s.to_string()).collect();
        c
    }

    fn make_instance(enabled: bool, caps: Option<ConfigCaps>) -> PluginInstanceConfig {
        let mut i = PluginInstanceConfig::default();
        i.enabled = enabled;
        i.capabilities = caps;
        i
    }

    #[test]
    fn resolve_capabilities_uses_global_defaults() {
        let global = config_caps(&["/data"], true, &["API_KEY"]);
        let caps = resolve_capabilities(&global, None);
        assert_eq!(caps.fs, vec!["/data"]);
        assert!(caps.network);
        assert_eq!(caps.env, vec!["API_KEY"]);
    }

    #[test]
    fn resolve_capabilities_with_full_override() {
        let global = config_caps(&["/data"], true, &["API_KEY"]);
        let per_plugin = config_caps(&[], false, &["SPECIAL"]);
        let instance = make_instance(true, Some(per_plugin));
        let caps = resolve_capabilities(&global, Some(&instance));
        assert!(caps.fs.is_empty());
        assert!(!caps.network);
        assert_eq!(caps.env, vec!["SPECIAL"]);
    }

    #[test]
    fn resolve_capabilities_partial_override_network_only() {
        let global = config_caps(&["."], false, &["DB_URL"]);
        let per_plugin = config_caps(&["."], true, &["DB_URL"]);
        let instance = make_instance(true, Some(per_plugin));
        let caps = resolve_capabilities(&global, Some(&instance));
        assert_eq!(caps.fs, vec!["."]);
        assert!(caps.network); // overridden from false → true
        assert_eq!(caps.env, vec!["DB_URL"]);
    }

    #[test]
    fn resolve_capabilities_no_instance_config() {
        let global = ConfigCaps::default();
        let caps = resolve_capabilities(&global, None);
        assert!(caps.fs.is_empty());
        assert!(!caps.network);
        assert!(caps.env.is_empty());
    }

    #[test]
    fn resolve_capabilities_instance_without_cap_override() {
        let global = config_caps(&["/tmp"], true, &["TOKEN"]);
        // Instance present but no capabilities override → global applies.
        let instance = make_instance(true, None);
        let caps = resolve_capabilities(&global, Some(&instance));
        assert_eq!(caps.fs, vec!["/tmp"]);
        assert!(caps.network);
        assert_eq!(caps.env, vec!["TOKEN"]);
    }
}
