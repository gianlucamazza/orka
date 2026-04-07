//! WASM Plugin Example
//!
//! Demonstrates how to load and execute WASM plugins as skills in Orka.
//!
//! ## Prerequisites
//!
//! 1. Build a WASM plugin (see `sdk/hello-plugin`)
//! 2. Set the path to the `.wasm` file
//!
//! ## Running
//!
//! ```bash
//! # First, build the hello-plugin example
//! cd sdk/hello-plugin
//! cargo build --target wasm32-wasip2 --release
//!
//! # Then run this example
//! cd ../../examples/wasm_plugin
//! export WASM_PLUGIN_PATH="../../sdk/hello-plugin/target/wasm32-wasip2/release/hello_plugin.wasm"
//! cargo run
//! ```

use anyhow::Result;
use orka_core::types::{SkillInput, SkillOutput};
use orka_wasm::{WasmComponent, WasmLimits};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("wasm_plugin=info")
        .init();

    info!("=== WASM Plugin Example ===\n");

    // Get plugin path from environment
    let plugin_path = std::env::var("WASM_PLUGIN_PATH")
        .unwrap_or_else(|_| {
            // Default fallback for the example
            "../../sdk/hello-plugin/target/wasm32-wasip2/release/hello_plugin.wasm".to_string()
        });

    let path = Path::new(&plugin_path);
    
    if !path.exists() {
        warn!("Plugin not found at: {}", plugin_path);
        println!("\nPlugin file not found.");
        println!("Please build the hello-plugin first:");
        println!("  cd sdk/hello-plugin");
        println!("  cargo build --target wasm32-wasip2 --release");
        println!("\nOr set WASM_PLUGIN_PATH to your plugin file.");
        return Ok(());
    }

    // Configure WASM runtime limits
    let limits = WasmLimits {
        max_memory_mb: 128,
        max_execution_time_secs: 30,
        max_fuel: Some(10_000_000_000), // ~10B instructions
    };

    info!("Loading WASM plugin from: {}", plugin_path);
    
    // Load the WASM component
    let component = match WasmComponent::from_file(path, limits).await {
        Ok(c) => {
            info!("Plugin loaded successfully!");
            info!("  Name: {}", c.name());
            info!("  Version: {}", c.version());
            info!("  Description: {}", c.description());
            c
        }
        Err(e) => {
            warn!("Failed to load plugin: {}", e);
            return Err(e.into());
        }
    };

    // Demonstrate plugin capabilities
    info!("\nPlugin Capabilities:");
    let caps = component.capabilities();
    info!("  Network: {}", caps.allow_network);
    info!("  Filesystem: {}", caps.allow_filesystem);
    info!("  Stdio: {}", caps.allow_stdio);
    info!("  Environment: {:?}", caps.allowed_env_vars);

    // Call the plugin
    info!("\nCalling plugin...");
    
    let input = SkillInput {
        args: [
            ("name".into(), serde_json::json!("Orka User")),
            ("message".into(), serde_json::json!("Hello from WASM!")),
        ]
        .into_iter()
        .collect(),
        context: None,
    };

    match component.execute(input).await {
        Ok(output) => {
            info!("Plugin executed successfully!");
            println!("\n=== Output ===");
            println!("{}", serde_json::to_string_pretty(&output.data)?);
        }
        Err(e) => {
            warn!("Plugin execution failed: {}", e);
        }
    }

    // Multiple calls demonstration
    info!("\nMaking multiple calls to demonstrate sandboxing...");
    
    for i in 1..=3 {
        let input = SkillInput {
            args: [
                ("iteration".into(), serde_json::json!(i)),
                ("data".into(), serde_json::json!(format!("Test data {}", i))),
            ]
            .into_iter()
            .collect(),
            context: None,
        };

        match component.execute(input).await {
            Ok(output) => {
                info!("Call {} succeeded: {:?}", i, output.data);
            }
            Err(e) => {
                warn!("Call {} failed: {}", i, e);
            }
        }
    }

    info!("\n=== Example Complete ===");
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_limits_default() {
        let limits = WasmLimits::default();
        assert!(limits.max_memory_mb > 0);
        assert!(limits.max_execution_time_secs > 0);
    }

    #[tokio::test]
    async fn test_plugin_loading_failure() {
        let limits = WasmLimits::default();
        let result = WasmComponent::from_file(Path::new("/nonexistent/plugin.wasm"), limits).await;
        assert!(result.is_err());
    }
}
