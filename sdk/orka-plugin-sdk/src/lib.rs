//! Orka Plugin SDK v2
//!
//! Plugin authors implement the `Plugin` trait and use the `export_plugin!` macro.
//!
//! # Example
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
//!             description: "A hello world plugin".into(),
//!             parameters_schema: "{}".into(),
//!         }
//!     }
//!
//!     fn execute(&self, input: PluginInput) -> Result<PluginOutput, String> {
//!         Ok(PluginOutput { data: "Hello!".into() })
//!     }
//! }
//!
//! orka_plugin_sdk::export_plugin!(HelloPlugin);
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// ABI version exported by v2 plugins.
pub const ABI_VERSION: i32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub parameters_schema: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInput {
    pub args: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginOutput {
    pub data: String,
}

/// Trait that plugin authors implement.
pub trait Plugin: Default {
    fn info(&self) -> PluginInfo;
    fn execute(&self, input: PluginInput) -> Result<PluginOutput, String>;

    /// Called once when the plugin is loaded by the host.
    fn init(&self) -> Result<(), String> {
        Ok(())
    }

    /// Called by the host before unloading the plugin.
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

        /// Return plugin metadata as JSON. Encoding: (ptr << 32) | len.
        #[no_mangle]
        pub extern "C" fn orka_plugin_info() -> i64 {
            let plugin = <$plugin_ty>::default();
            let info = <$plugin_ty as $crate::Plugin>::info(&plugin);
            let json = serde_json::to_vec(&info).unwrap();
            let len = json.len() as i64;
            let ptr = json.as_ptr() as i64;
            std::mem::forget(json);
            (ptr << 32) | len
        }

        /// Execute the plugin with JSON input. Encoding: (ptr << 32) | len.
        #[no_mangle]
        pub extern "C" fn orka_plugin_execute(ptr: u32, len: u32) -> i64 {
            // SAFETY: The host allocated this memory via `orka_alloc` and wrote `len` bytes
            // starting at `ptr`. The memory is valid for the duration of this call.
            let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
            let input: $crate::PluginInput = match serde_json::from_slice(input_bytes) {
                Ok(v) => v,
                Err(e) => {
                    let err =
                        serde_json::to_vec(&Err::<$crate::PluginOutput, String>(e.to_string()))
                            .unwrap();
                    let elen = err.len() as i64;
                    let eptr = err.as_ptr() as i64;
                    std::mem::forget(err);
                    return (eptr << 32) | elen;
                }
            };
            let plugin = <$plugin_ty>::default();
            let result = <$plugin_ty as $crate::Plugin>::execute(&plugin, input);
            let json = serde_json::to_vec(&result).unwrap();
            let rlen = json.len() as i64;
            let rptr = json.as_ptr() as i64;
            std::mem::forget(json);
            (rptr << 32) | rlen
        }

        /// Allocate `len` bytes of guest memory. Returns the pointer.
        #[no_mangle]
        pub extern "C" fn orka_alloc(len: u32) -> u32 {
            let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
            // SAFETY: layout is valid.
            unsafe { std::alloc::alloc(layout) as u32 }
        }

        /// Free `len` bytes at `ptr` previously allocated by `orka_alloc`.
        #[no_mangle]
        pub extern "C" fn orka_dealloc(ptr: u32, len: u32) {
            if len == 0 {
                return;
            }
            let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
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
