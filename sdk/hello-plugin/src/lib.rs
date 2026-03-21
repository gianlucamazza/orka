use orka_plugin_sdk_component::{Plugin, export_plugin};

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
        let name = input
            .args
            .iter()
            .find(|(k, _)| k == "name")
            .map(|(_, v)| v.trim_matches('"').to_string())
            .unwrap_or_else(|| "world".to_string());

        Ok(orka_plugin_sdk_component::PluginOutput {
            data: format!("Hello, {name}!"),
        })
    }
}

export_plugin!(HelloPlugin);
