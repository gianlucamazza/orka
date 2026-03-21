//! Guest-side SDK for writing Orka WASM Component Model plugins.
//!
//! # Usage
//!
//! Add to your plugin's `Cargo.toml`:
//!
//! ```toml
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [dependencies]
//! orka-plugin-sdk-component = { path = "..." }
//! wit-bindgen = "0.51"
//! ```
//!
//! Then implement the [`Plugin`] trait and register it:
//!
//! ```rust,ignore
//! use orka_plugin_sdk_component::{Plugin, PluginInfo, PluginInput, PluginOutput, export_plugin};
//!
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn info() -> PluginInfo {
//!         PluginInfo {
//!             name: "hello".to_string(),
//!             description: "Returns a greeting".to_string(),
//!             parameters_schema: r#"{"type":"object","properties":{}}"#.to_string(),
//!         }
//!     }
//!
//!     fn execute(input: PluginInput) -> Result<PluginOutput, String> {
//!         let name = input.args.iter()
//!             .find(|(k, _)| k == "name")
//!             .map(|(_, v)| v.trim_matches('"'))
//!             .unwrap_or("world");
//!         Ok(PluginOutput { data: format!("Hello, {name}!") })
//!     }
//! }
//!
//! export_plugin!(Hello);
//! ```

/// Static metadata returned by [`Plugin::info`].
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub parameters_schema: String,
}

/// Arguments passed to [`Plugin::execute`].
pub struct PluginInput {
    pub args: Vec<(String, String)>,
}

/// Successful output from [`Plugin::execute`].
pub struct PluginOutput {
    pub data: String,
}

/// Trait that every Orka plugin must implement.
///
/// The `init` and `cleanup` methods have default no-op implementations.
pub trait Plugin {
    /// Return static metadata about this plugin.
    fn info() -> PluginInfo;

    /// One-time initialisation. Called once after loading.
    fn init() -> Result<(), String> {
        Ok(())
    }

    /// Execute the skill with the given arguments.
    fn execute(input: PluginInput) -> Result<PluginOutput, String>;

    /// Release any resources. Called once before unloading.
    fn cleanup() -> Result<(), String> {
        Ok(())
    }
}

/// Register a [`Plugin`] implementation as the WIT-exported WASM component.
///
/// This macro generates the WIT bindings inline and wires up the WASM exports.
/// The plugin crate must also add `wit-bindgen` as a direct dependency.
///
/// Call this once at the bottom of your plugin crate.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        ::wit_bindgen::generate!({
            inline: r#"
package orka:plugin@2.0.0;

world plugin {
    record plugin-info {
        name: string,
        description: string,
        parameters-schema: string,
    }

    record plugin-input {
        args: list<tuple<string, string>>,
    }

    record plugin-output {
        data: string,
    }

    export info: func() -> plugin-info;
    export init: func() -> result<_, string>;
    export execute: func(input: plugin-input) -> result<plugin-output, string>;
    export cleanup: func() -> result<_, string>;
}
            "#,
            world: "plugin",
        });

        struct __OrkaGuest;

        impl Guest for __OrkaGuest {
            fn info() -> PluginInfo {
                let i = <$plugin as $crate::Plugin>::info();
                PluginInfo {
                    name: i.name,
                    description: i.description,
                    parameters_schema: i.parameters_schema,
                }
            }

            fn init() -> ::core::result::Result<(), ::std::string::String> {
                <$plugin as $crate::Plugin>::init()
            }

            fn execute(
                input: PluginInput,
            ) -> ::core::result::Result<PluginOutput, ::std::string::String> {
                let sdk_input = $crate::PluginInput { args: input.args };
                <$plugin as $crate::Plugin>::execute(sdk_input)
                    .map(|o| PluginOutput { data: o.data })
            }

            fn cleanup() -> ::core::result::Result<(), ::std::string::String> {
                <$plugin as $crate::Plugin>::cleanup()
            }
        }

        export!(__OrkaGuest with_types_in self);
    };
}
