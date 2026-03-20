//! Orka Plugin SDK — WASM plugin ABI v2.
//!
//! This crate provides everything plugin authors need to build Orka-compatible
//! WebAssembly plugins. Plugins are compiled to `wasm32-wasip1` and loaded at
//! runtime by the Orka skill engine.
//!
//! # Getting started
//!
//! 1. Create a new library crate and add this SDK as a dependency.
//! 2. Implement the [`Plugin`] trait on a `Default` struct.
//! 3. Call [`export_plugin!`] to generate the required WASM FFI exports.
//! 4. Build with `cargo build --target wasm32-wasip1 --release`.
//! 5. Place the `.wasm` file in your Orka `plugins.dir` directory.
//!
//! # Example
//!
//! ```ignore
//! use orka_plugin_sdk::{Plugin, PluginInfo, PluginInput, PluginOutput};
//!
//! #[derive(Default)]
//! struct HelloPlugin;
//!
//! impl Plugin for HelloPlugin {
//!     fn info(&self) -> PluginInfo {
//!         PluginInfo {
//!             name: "hello".into(),
//!             description: "Greet the caller by name".into(),
//!             // JSON Schema for the `args` map your plugin accepts.
//!             parameters_schema: r#"{
//!                 "type": "object",
//!                 "properties": {
//!                     "name": { "type": "string", "description": "Name to greet" }
//!                 },
//!                 "required": ["name"]
//!             }"#.into(),
//!         }
//!     }
//!
//!     fn execute(&self, input: PluginInput) -> Result<PluginOutput, String> {
//!         let name = input.args.get("name")
//!             .and_then(|v| v.as_str())
//!             .unwrap_or("world");
//!         Ok(PluginOutput { data: format!("Hello, {name}!") })
//!     }
//! }
//!
//! orka_plugin_sdk::export_plugin!(HelloPlugin);
//! ```
//!
//! # ABI contract
//!
//! The [`export_plugin!`] macro generates the following `extern "C"` exports that
//! the Orka host calls:
//!
//! | Symbol | Signature | Description |
//! |---|---|---|
//! | `orka_abi_version` | `() -> i32` | Returns `2` for ABI v2 |
//! | `orka_plugin_init` | `() -> i32` | Called once on load; `0` = OK |
//! | `orka_plugin_cleanup` | `() -> i32` | Called before unload; `0` = OK |
//! | `orka_plugin_info` | `() -> i64` | Returns `(ptr << 32) \| len` for JSON metadata |
//! | `orka_plugin_execute` | `(ptr: u32, len: u32) -> i64` | Execute with JSON input; returns `(ptr << 32) \| len` |
//! | `orka_alloc` | `(len: u32) -> u32` | Allocate guest memory |
//! | `orka_dealloc` | `(ptr: u32, len: u32)` | Free guest memory |

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// ABI version exported by v2 plugins.
pub const ABI_VERSION: i32 = 2;

/// Metadata returned by a plugin describing its identity and parameter contract.
///
/// The `parameters_schema` field must be a valid [JSON Schema](https://json-schema.org/)
/// string. Orka uses it to validate input and to present the plugin's interface to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Unique skill name registered in the Orka skill registry (e.g. `"web_search"`).
    pub name: String,
    /// Human-readable description shown to the LLM as part of the tool definition.
    pub description: String,
    /// JSON Schema for the `args` object passed to [`Plugin::execute`].
    pub parameters_schema: String,
}

/// Input passed to [`Plugin::execute`] on each invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInput {
    /// Named arguments extracted from the LLM tool call.
    /// Values are arbitrary JSON.
    pub args: HashMap<String, serde_json::Value>,
}

/// Output returned from a successful [`Plugin::execute`] call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginOutput {
    /// Result string sent back to the LLM as the tool result.
    pub data: String,
}

/// Core trait implemented by all Orka WASM plugins.
///
/// Plugin types must also implement [`Default`] so the host can instantiate them
/// without arguments. State should not be held across calls — plugins are stateless
/// by design.
pub trait Plugin: Default {
    /// Return static metadata about this plugin.
    ///
    /// Called once by the host after [`Plugin::init`] succeeds.
    fn info(&self) -> PluginInfo;

    /// Execute the plugin with the given input and return a result.
    ///
    /// Return `Err(String)` to signal a skill error; the message is forwarded
    /// to the LLM as a tool error result.
    fn execute(&self, input: PluginInput) -> Result<PluginOutput, String>;

    /// Called once when the plugin is loaded by the host.
    ///
    /// Use this to validate configuration or warm up resources. The default
    /// implementation does nothing and returns `Ok(())`.
    fn init(&self) -> Result<(), String> {
        Ok(())
    }

    /// Called by the host before unloading the plugin.
    ///
    /// Use this to flush buffers or release external resources. The default
    /// implementation does nothing and returns `Ok(())`.
    fn cleanup(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Macro to export a plugin. Generates the WASM-compatible FFI functions (ABI v2).
#[macro_export]
macro_rules! export_plugin {
    ($plugin_ty:ty) => {
        #[cfg(target_pointer_width = "32")]
        const _: () = assert!(
            std::mem::size_of::<*const u8>() <= 4,
            "pointer must fit in 32 bits for WASM plugin ABI"
        );

        /// Return ABI version (2).
        #[no_mangle]
        pub extern "C" fn orka_abi_version() -> i32 {
            $crate::ABI_VERSION
        }

        /// Called by the host after loading. Returns 0 on success, 1 on error.
        #[no_mangle]
        pub extern "C" fn orka_plugin_init() -> i32 {
            let plugin = <$plugin_ty>::default();
            match <$plugin_ty as $crate::Plugin>::init(&plugin) {
                Ok(()) => 0,
                Err(_) => 1,
            }
        }

        /// Called by the host before unloading. Returns 0 on success, 1 on error.
        #[no_mangle]
        pub extern "C" fn orka_plugin_cleanup() -> i32 {
            let plugin = <$plugin_ty>::default();
            match <$plugin_ty as $crate::Plugin>::cleanup(&plugin) {
                Ok(()) => 0,
                Err(_) => 1,
            }
        }

        /// Return plugin metadata as JSON. Encoding: (ptr << 32) | len, or 0 on error.
        #[no_mangle]
        pub extern "C" fn orka_plugin_info() -> i64 {
            let plugin = <$plugin_ty>::default();
            let info = <$plugin_ty as $crate::Plugin>::info(&plugin);
            let Ok(json) = serde_json::to_vec(&info) else {
                return 0;
            };
            let len = json.len() as i64;
            let ptr = json.as_ptr() as i64;
            std::mem::forget(json);
            (ptr << 32) | len
        }

        /// Execute the plugin with JSON input. Encoding: (ptr << 32) | len, or 0 on error.
        #[no_mangle]
        pub extern "C" fn orka_plugin_execute(ptr: u32, len: u32) -> i64 {
            // SAFETY: The host allocated this memory via `orka_alloc` and wrote `len` bytes
            // starting at `ptr`. The memory is valid for the duration of this call.
            let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
            let input: $crate::PluginInput = match serde_json::from_slice(input_bytes) {
                Ok(v) => v,
                Err(e) => {
                    let Ok(err) =
                        serde_json::to_vec(&Err::<$crate::PluginOutput, String>(e.to_string()))
                    else {
                        return 0;
                    };
                    let elen = err.len() as i64;
                    let eptr = err.as_ptr() as i64;
                    std::mem::forget(err);
                    return (eptr << 32) | elen;
                }
            };
            let plugin = <$plugin_ty>::default();
            let result = <$plugin_ty as $crate::Plugin>::execute(&plugin, input);
            let Ok(json) = serde_json::to_vec(&result) else {
                return 0;
            };
            let rlen = json.len() as i64;
            let rptr = json.as_ptr() as i64;
            std::mem::forget(json);
            (rptr << 32) | rlen
        }

        /// Allocate `len` bytes of guest memory. Returns the pointer, or 0 on failure.
        #[no_mangle]
        pub extern "C" fn orka_alloc(len: u32) -> u32 {
            let Ok(layout) = std::alloc::Layout::from_size_align(len as usize, 1) else {
                return 0;
            };
            // SAFETY: layout is valid (size > 0, alignment = 1).
            unsafe { std::alloc::alloc(layout) as u32 }
        }

        /// Free `len` bytes at `ptr` previously allocated by `orka_alloc`.
        #[no_mangle]
        pub extern "C" fn orka_dealloc(ptr: u32, len: u32) {
            if len == 0 {
                return;
            }
            let Ok(layout) = std::alloc::Layout::from_size_align(len as usize, 1) else {
                return;
            };
            // SAFETY: ptr was allocated by orka_alloc with the same layout.
            unsafe { std::alloc::dealloc(ptr as *mut u8, layout) }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_info_serde_roundtrip() {
        let info = PluginInfo {
            name: "test".into(),
            description: "A test plugin".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: PluginInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, info.name);
        assert_eq!(decoded.description, info.description);
        assert_eq!(decoded.parameters_schema, info.parameters_schema);
    }

    #[test]
    fn plugin_input_serde_roundtrip() {
        let mut args = HashMap::new();
        args.insert("key".to_string(), serde_json::json!("value"));
        args.insert("count".to_string(), serde_json::json!(42));
        let input = PluginInput { args };
        let json = serde_json::to_string(&input).unwrap();
        let decoded: PluginInput = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.args["key"], serde_json::json!("value"));
        assert_eq!(decoded.args["count"], serde_json::json!(42));
    }

    #[test]
    fn plugin_output_serde_roundtrip() {
        let output = PluginOutput {
            data: "result data".into(),
        };
        let json = serde_json::to_string(&output).unwrap();
        let decoded: PluginOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.data, output.data);
    }
}
