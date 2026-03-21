use orka_core::{Error, Result};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::config::WasmLimits;
use crate::engine::WasmEngine;

/// Capabilities granted to a plugin instance (deny-by-default).
#[derive(Debug, Clone, Default)]
pub struct PluginCapabilities {
    /// Allowed environment variable names (empty = deny all).
    pub env: Vec<String>,
    /// Allowed host filesystem paths to pre-open (empty = deny all).
    pub fs: Vec<String>,
    /// If true, allow TCP/UDP networking.
    pub network: bool,
}

// Isolated submodule so that `bindgen!`'s generated `Result<T, E>` usage
// doesn't conflict with our single-param `orka_core::Result<T>` alias.
mod bindings {
    wasmtime::component::bindgen!({
        path: "wit/orka-plugin.wit",
        world: "plugin",
    });
}

pub use bindings::{Plugin, PluginInfo, PluginInput, PluginOutput};

struct PluginState {
    ctx: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
}

impl WasiView for PluginState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

/// A compiled WASM Component. Cheap to clone, thread-safe, reusable across calls.
#[derive(Clone)]
pub struct WasmComponent {
    pub(crate) component: Component,
    pub(crate) engine: WasmEngine,
}

impl WasmComponent {
    /// Load and compile a WASM component from bytes.
    pub fn compile(engine: &WasmEngine, bytes: &[u8]) -> Result<Self> {
        let component = Component::new(&engine.0, bytes)
            .map_err(|e| Error::sandbox_msg(format!("failed to compile WASM component: {e}")))?;
        Ok(Self {
            component,
            engine: engine.clone(),
        })
    }

    /// Execute `info` → `init` → `execute(args)` → `cleanup` in a fresh store.
    ///
    /// Returns the plugin metadata and the execute output string.
    pub fn run(
        &self,
        args: Vec<(String, String)>,
        caps: &PluginCapabilities,
        limits: &WasmLimits,
    ) -> Result<(PluginInfo, String)> {
        let (plugin, mut store) = self.instantiate(caps, limits)?;

        let info = plugin
            .call_info(&mut store)
            .map_err(|e| Error::sandbox_msg(format!("plugin info() trap: {e}")))?;

        plugin
            .call_init(&mut store)
            .map_err(|e| Error::sandbox_msg(format!("plugin init() trap: {e}")))?
            .map_err(|e| Error::Skill(format!("plugin init() error: {e}")))?;

        let input = PluginInput { args };
        let output = plugin
            .call_execute(&mut store, &input)
            .map_err(|e| Error::sandbox_msg(format!("plugin execute() trap: {e}")))?
            .map_err(|e| Error::Skill(format!("plugin execute() error: {e}")))?;

        let _ = plugin.call_cleanup(&mut store);

        Ok((info, output.data))
    }

    /// Probe plugin metadata without executing.
    pub fn probe_info(&self, caps: &PluginCapabilities, limits: &WasmLimits) -> Result<PluginInfo> {
        let (plugin, mut store) = self.instantiate(caps, limits)?;
        plugin
            .call_info(&mut store)
            .map_err(|e| Error::sandbox_msg(format!("plugin info() trap: {e}")))
    }

    fn instantiate(
        &self,
        caps: &PluginCapabilities,
        limits: &WasmLimits,
    ) -> Result<(Plugin, Store<PluginState>)> {
        let mut store = self.build_store(caps, limits)?;
        let mut linker = Linker::<PluginState>::new(&self.engine.0);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| Error::sandbox_msg(format!("failed to add WASI to linker: {e}")))?;

        let plugin = Plugin::instantiate(&mut store, &self.component, &linker)
            .map_err(|e| Error::sandbox_msg(format!("failed to instantiate component: {e}")))?;

        Ok((plugin, store))
    }

    fn build_store(
        &self,
        caps: &PluginCapabilities,
        limits: &WasmLimits,
    ) -> Result<Store<PluginState>> {
        let mut builder = WasiCtxBuilder::new();

        // Env: only inject the explicitly allowed variables.
        for name in &caps.env {
            if let Ok(val) = std::env::var(name) {
                builder.env(name, &val);
            }
        }

        // FS: pre-open the listed host directories under the same guest path.
        for path in &caps.fs {
            builder
                .preopened_dir(path, path, DirPerms::all(), FilePerms::all())
                .map_err(|e| {
                    Error::sandbox_msg(format!("failed to pre-open plugin fs path '{path}': {e}"))
                })?;
        }

        // Network: opt-in (deny by default).
        if caps.network {
            builder.allow_tcp(true).allow_udp(true);
        }

        let ctx = builder.build();
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .build();

        let mut store = Store::new(
            &self.engine.0,
            PluginState {
                ctx,
                table: ResourceTable::new(),
                limits: store_limits,
            },
        );
        store.limiter(|s| &mut s.limits);

        if let Some(fuel) = limits.fuel {
            store
                .set_fuel(fuel)
                .map_err(|e| Error::sandbox_msg(format!("failed to set fuel: {e}")))?;
        }

        Ok(store)
    }
}
