//! Tests for the WASM Component Model path.

use orka_wasm::{PluginCapabilities, WasmEngine, WasmLimits};

// ---------------------------------------------------------------------------
// Negative path: reject non-component binaries
// ---------------------------------------------------------------------------

/// A core module (WASM 1.0) must NOT be accepted as a component.
#[test]
fn compile_component_rejects_core_module() {
    let engine = WasmEngine::new().unwrap();
    // Minimal valid WASM 1.0 module — magic + version 0x01
    let core_module = b"\x00asm\x01\x00\x00\x00";
    let result = engine.compile_component(core_module);
    assert!(
        result.is_err(),
        "compile_component should reject a core WASM module"
    );
}

/// Garbage bytes must not compile as a component.
#[test]
fn compile_component_rejects_garbage() {
    let engine = WasmEngine::new().unwrap();
    let result = engine.compile_component(b"not a component at all");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// PluginCapabilities defaults
// ---------------------------------------------------------------------------

/// Default capabilities deny everything.
#[test]
fn default_capabilities_deny_all() {
    let caps = PluginCapabilities::default();
    assert!(caps.env.is_empty(), "default: no env vars");
    assert!(caps.fs.is_empty(), "default: no fs paths");
    assert!(!caps.network, "default: no network");
}

/// Capabilities with only env set.
#[test]
fn capabilities_env_only() {
    let caps = PluginCapabilities {
        env: vec!["API_KEY".to_string(), "LOG_LEVEL".to_string()],
        ..Default::default()
    };
    assert_eq!(caps.env.len(), 2);
    assert!(caps.fs.is_empty());
    assert!(!caps.network);
}

// ---------------------------------------------------------------------------
// Integration: round-trip with a pre-compiled component fixture
//
// This test is ignored by default because it requires a compiled `.wasm`
// Component Model binary. To run it:
//
//   1. Build the hello plugin:
//      cargo build -p hello-plugin --target wasm32-wasip2 --release
//      cp target/wasm32-wasip2/release/hello_plugin.wasm \
//         crates/orka-wasm/tests/fixtures/hello-plugin.wasm
//
//   2. Run:
//      cargo test -p orka-wasm -- --ignored component_round_trip
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires pre-compiled fixture: crates/orka-wasm/tests/fixtures/hello-plugin.wasm"]
fn component_round_trip() {
    let fixture = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/hello-plugin.wasm"
    ));
    if !fixture.exists() {
        eprintln!("fixture not found at {}", fixture.display());
        return;
    }

    let bytes = std::fs::read(fixture).expect("read fixture");
    let engine = WasmEngine::new().unwrap();
    let component = engine.compile_component(&bytes).expect("compile component");

    let caps = PluginCapabilities::default();
    let limits = WasmLimits::default();

    let info = component.probe_info(&caps, &limits).expect("probe_info");
    assert!(!info.name.is_empty(), "plugin name must not be empty");
    assert!(
        !info.description.is_empty(),
        "plugin description must not be empty"
    );

    let (_, data) = component.run(vec![], &caps, &limits).expect("run plugin");
    assert!(!data.is_empty(), "plugin output must not be empty");
}
