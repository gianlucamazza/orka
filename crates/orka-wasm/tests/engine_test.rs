#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::redundant_clone
)]

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

// WASI preview1 `fd_prestat_get` / `path_open`. The P1 sandbox never preopens
// host directories, so a guest must not observe a directory fd or read host
// files such as /etc/passwd.
const PRESTAT_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_prestat_get"
    (func $fd_prestat_get (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "probe") (param $fd i32) (result i32)
    (call $fd_prestat_get (local.get $fd) (i32.const 0))
  )
)
"#;

const PATH_OPEN_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "etc/passwd")
  (func (export "open_passwd") (result i32)
    ;; path_open(dirfd=3, lookupflags=0, path="etc/passwd", oflags=0,
    ;;           rights=fd_read, inheriting=0, fdflags=0, result@200)
    (call $path_open
      (i32.const 3) (i32.const 0)
      (i32.const 0) (i32.const 10)
      (i32.const 0)
      (i64.const 1) (i64.const 0)
      (i32.const 0)
      (i32.const 200))
  )
)
"#;

/// Default P1 instances have no directory preopens (stdio pipes only).
#[test]
fn guest_has_no_filesystem_preopens() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(PRESTAT_WAT.as_bytes()).unwrap();
    let mut instance = WasmInstance::build(&module, &WasmLimits::default(), None, &[]).unwrap();
    for fd in 0..32i32 {
        let errno: i32 = instance.call("probe", fd).unwrap();
        assert_ne!(
            errno, 0,
            "fd {fd} must not be a WASI directory preopen (errno 0 = success)"
        );
    }
}

/// A guest cannot `path_open` host files when no dirs are preopened.
#[test]
fn guest_cannot_read_host_filesystem() {
    let engine = WasmEngine::new().unwrap();
    let module = engine.compile(PATH_OPEN_WAT.as_bytes()).unwrap();
    let mut instance = WasmInstance::build(&module, &WasmLimits::default(), None, &[]).unwrap();
    let errno: i32 = instance.call("open_passwd", ()).unwrap();
    assert_ne!(
        errno, 0,
        "path_open(etc/passwd) must fail without a preopen"
    );

    // Even if the guest wrote something at the result slot, it must not be a
    // usable host fd and guest memory must not contain /etc/passwd contents.
    let peek = instance.read_memory(0, 64).unwrap();
    let peek_str = String::from_utf8_lossy(&peek);
    assert!(
        !peek_str.contains("root:"),
        "guest memory must not contain host /etc/passwd contents"
    );
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
