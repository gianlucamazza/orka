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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub parameters_schema: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInput {
    pub args: Vec<(String, String)>,
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
        #[no_mangle]
        pub extern "C" fn orka_plugin_info() -> u64 {
            let plugin = <$plugin_ty>::default();
            let info = <$plugin_ty as $crate::Plugin>::info(&plugin);
            let json = serde_json::to_vec(&info).unwrap();
            let len = json.len() as u64;
            let ptr = json.as_ptr() as u64;
            std::mem::forget(json);
            (ptr << 32) | len
        }

        #[no_mangle]
        pub extern "C" fn orka_plugin_execute(ptr: u32, len: u32) -> u64 {
            let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
            let input: $crate::PluginInput = match serde_json::from_slice(input_bytes) {
                Ok(v) => v,
                Err(e) => {
                    let err =
                        serde_json::to_vec(&Err::<$crate::PluginOutput, String>(e.to_string()))
                            .unwrap();
                    let elen = err.len() as u64;
                    let eptr = err.as_ptr() as u64;
                    std::mem::forget(err);
                    return (eptr << 32) | elen;
                }
            };
            let plugin = <$plugin_ty>::default();
            let result = <$plugin_ty as $crate::Plugin>::execute(&plugin, input);
            let json = serde_json::to_vec(&result).unwrap();
            let rlen = json.len() as u64;
            let rptr = json.as_ptr() as u64;
            std::mem::forget(json);
            (rptr << 32) | rlen
        }

        #[no_mangle]
        pub extern "C" fn orka_alloc(len: u32) -> u32 {
            let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
            unsafe { std::alloc::alloc(layout) as u32 }
        }
    };
}
