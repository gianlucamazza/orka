//! Example Orka WASM plugin that returns a greeting.
// The export_plugin! macro generates undocumented types; allow missing_docs for this example.
#![allow(missing_docs, clippy::same_length_and_capacity)]

use orka_plugin_sdk_component::{Plugin, export_plugin};

/// Example plugin: greets the caller by name.
struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn info() -> orka_plugin_sdk_component::PluginInfo {
        orka_plugin_sdk_component::PluginInfo {
            name: "hello".to_string(),
            description: "Returns a greeting for the given name".to_string(),
            parameters_schema: r#"{
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name to greet" }
                }
            }"#
            .to_string(),
        }
    }

    fn execute(
        input: orka_plugin_sdk_component::PluginInput,
    ) -> Result<orka_plugin_sdk_component::PluginOutput, String> {
        let name = input.get_str("name").unwrap_or("world").to_string();

        Ok(orka_plugin_sdk_component::PluginOutput {
            data: format!("Hello, {name}!"),
        })
    }
}

export_plugin!(HelloPlugin);
