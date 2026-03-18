//! Orka Plugin SDK
//!
//! Plugin authors implement the `Plugin` trait and use the `export_plugin!` macro.
//!
//! # Example
//! ```ignore
//! use orka_plugin_sdk::{Plugin, PluginInfo, PluginInput, PluginOutput};
//!
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
}

/// Macro to export a plugin. Generates the WASM-compatible FFI functions.
#[macro_export]
macro_rules! export_plugin {
    ($plugin_ty:ty) => {
        // SAFETY: We encode the return value as a u64 packing (ptr << 32) | len.
        // This requires that pointers fit in 32 bits (WASM is a 32-bit target).
        #[cfg(target_pointer_width = "32")]
        const _: () = assert!(
            std::mem::size_of::<*const u8>() <= 4,
            "pointer must fit in 32 bits for WASM plugin ABI"
        );

        #[no_mangle]
        pub extern "C" fn orka_plugin_info() -> u64 {
            let plugin = <$plugin_ty>::default();
            let info = <$plugin_ty as $crate::Plugin>::info(&plugin);
            let json = serde_json::to_vec(&info).unwrap();
            let len = json.len() as u64;
            // SAFETY: json is a valid Vec allocation. We forget it to prevent deallocation
            // and return the pointer packed with the length for the host to read via WASM memory.
            let ptr = json.as_ptr() as u64;
            std::mem::forget(json);
            (ptr << 32) | len
        }

        #[no_mangle]
        pub extern "C" fn orka_plugin_execute(ptr: u32, len: u32) -> u64 {
            // SAFETY: The host allocated this memory via `orka_alloc` and wrote `len` bytes
            // starting at `ptr`. The memory is valid for the duration of this call.
            let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
            let input: $crate::PluginInput = match serde_json::from_slice(input_bytes) {
                Ok(v) => v,
                Err(e) => {
                    let err =
                        serde_json::to_vec(&Err::<$crate::PluginOutput, String>(e.to_string()))
                            .unwrap();
                    let elen = err.len() as u64;
                    // SAFETY: err is a valid Vec allocation; forgotten to transfer ownership to host.
                    let eptr = err.as_ptr() as u64;
                    std::mem::forget(err);
                    return (eptr << 32) | elen;
                }
            };
            let plugin = <$plugin_ty>::default();
            let result = <$plugin_ty as $crate::Plugin>::execute(&plugin, input);
            let json = serde_json::to_vec(&result).unwrap();
            let rlen = json.len() as u64;
            // SAFETY: json is a valid Vec allocation; forgotten to transfer ownership to host.
            let rptr = json.as_ptr() as u64;
            std::mem::forget(json);
            (rptr << 32) | rlen
        }

        #[no_mangle]
        pub extern "C" fn orka_alloc(len: u32) -> u32 {
            let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
            // SAFETY: layout is valid (size > 0 is not guaranteed but alloc handles it).
            unsafe { std::alloc::alloc(layout) as u32 }
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
