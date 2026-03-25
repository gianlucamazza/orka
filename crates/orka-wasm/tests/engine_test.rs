#![allow(missing_docs)]

//! Integration tests for `WasmEngine`, `WasmModule`, and `WasmInstance`.

use orka_wasm::{WasmEngine, WasmInstance, WasmLimits};

// Minimal WAT module that exports a single `add(i32, i32) -> i32` function.
const ADD_WAT: &str = r#"
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add
  )
)
"#;

// WAT module with a tight infinite loop — used to test fuel exhaustion.
const LOOP_WAT: &str = r#"
(module
  (func (export "spin")
    (block $b
      (loop $l
        (br $l)
      )
    )
  )
)
"#;

// ---------------------------------------------------------------------------
// Engine & compilation
// ---------------------------------------------------------------------------

#[test]
fn engine_creates_successfully() {
    assert!(WasmEngine::new().is_ok());
}

#[test]
fn compile_invalid_bytes_returns_error() {
    let engine = WasmEngine::new().unwrap();
    let result = engine.compile(b"not valid wasm");
    assert!(result.is_err());
}

#[test]
fn compile_valid_wat_succeeds() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(ADD_WAT.as_bytes());
    assert!(module.is_ok());
}

/// The same compiled module can be cloned and used independently.
#[test]
fn compiled_module_is_cloneable() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(ADD_WAT.as_bytes()).unwrap();
    let _clone = module.clone();
}

// ---------------------------------------------------------------------------
// Instance & execution
// ---------------------------------------------------------------------------

#[test]
fn instantiate_and_call_add_function() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(ADD_WAT.as_bytes()).unwrap();
    let mut instance = WasmInstance::build(&module, &WasmLimits::default(), None, &[]).unwrap();
    let result: i32 = instance.call("add", (3i32, 4i32)).unwrap();
    assert_eq!(result, 7);
}

#[test]
fn call_missing_export_returns_error() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(ADD_WAT.as_bytes()).unwrap();
    let mut instance = WasmInstance::build(&module, &WasmLimits::default(), None, &[]).unwrap();
    let err = instance.call::<(), ()>("nonexistent", ());
    assert!(err.is_err());
}

#[test]
fn fuel_exhaustion_returns_error() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(LOOP_WAT.as_bytes()).unwrap();

    let limits = WasmLimits {
        fuel: Some(1_000), // very low — will exhaust quickly
        ..WasmLimits::default()
    };
    let mut instance = WasmInstance::build(&module, &limits, None, &[]).unwrap();
    // The tight loop will consume all fuel and the call must fail
    let result = instance.call::<(), ()>("spin", ());
    assert!(result.is_err());
}

#[test]
fn into_output_returns_empty_for_module_with_no_io() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(ADD_WAT.as_bytes()).unwrap();
    let mut instance = WasmInstance::build(&module, &WasmLimits::default(), None, &[]).unwrap();
    let _: i32 = instance.call("add", (1i32, 2i32)).unwrap();
    let (stdout, stderr) = instance.into_output();
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

/// Build multiple instances from the same module — verifies the module is
/// reusable.
#[test]
fn module_can_be_instantiated_multiple_times() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(ADD_WAT.as_bytes()).unwrap();

    for n in 0..3i32 {
        let mut instance = WasmInstance::build(&module, &WasmLimits::default(), None, &[]).unwrap();
        let result: i32 = instance.call("add", (n, n)).unwrap();
        assert_eq!(result, n * 2);
    }
}
