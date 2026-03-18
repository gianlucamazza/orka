use std::path::Path;
use std::sync::Arc;

use orka_core::traits::Skill;
use tracing::{info, warn};

/// Scan `dir` for `.wasm` files and load each one as a [`WasmPluginSkill`].
///
/// Missing or empty directories are silently skipped. Files that fail to load
/// are logged as warnings and skipped rather than aborting the whole scan.
pub fn load_plugins(dir: &Path) -> orka_core::Result<Vec<Arc<dyn Skill>>> {
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
            match super::wasm_plugin::WasmPluginSkill::load(&path) {
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
