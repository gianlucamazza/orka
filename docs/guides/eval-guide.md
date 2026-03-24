# Evaluation Framework (orka-eval)

`orka-eval` is a lightweight framework for testing skill behaviour offline using
TOML-based scenario files. It runs directly against the skill registry without
starting a server.

---

## Quick Start

```bash
# Run all scenarios in the evals/ directory
cargo run -p orka-eval -- evals/

# Run scenarios for a specific skill
cargo run -p orka-eval -- evals/ --skill echo
```

---

## Scenario File Format

Scenario files use the `.eval.toml` extension and live in any directory
(by convention `evals/` at the project root).

```toml
# evals/echo.eval.toml

skill = "echo"   # optional: inferred from filename if omitted

[[scenarios]]
name        = "basic_echo"
description = "Skill echoes the input message back"

[scenarios.input]
message = "hello world"

[scenarios.expected]
is_ok          = true
contains       = ["hello world"]
max_duration_ms = 500

[[scenarios]]
name        = "empty_message"
description = "Empty message is handled gracefully"

[scenarios.input]
message = ""

[scenarios.expected]
is_ok = true
```

---

## Assertion Reference

| Field             | Type     | Description                                            |
| ----------------- | -------- | ------------------------------------------------------ |
| `is_ok`           | bool     | `true` → skill must succeed; `false` → skill must fail |
| `contains`        | [string] | Each substring must appear in the output text          |
| `not_contains`    | [string] | No substring may appear in the output text             |
| `format`          | "json"   | Output must deserialise as valid JSON                  |
| `output_matches`  | regex    | Output must match the regular expression               |
| `max_duration_ms` | integer  | Execution must complete within this many milliseconds  |

All specified assertions must pass for a scenario to be marked passed.

---

## Using EvalRunner Programmatically

```rust
use std::path::Path;
use std::sync::Arc;
use orka_eval::EvalRunner;
use orka_skills::SkillRegistry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut registry = SkillRegistry::new();
    registry.register(Arc::new(my_crate::MySkill));

    let runner = EvalRunner::new(Arc::new(registry));

    // Run all scenarios in evals/
    let report = runner.run_dir(Path::new("evals"), None).await?;

    println!("{}/{} passed in {:?}", report.passed, report.total, report.duration);

    for result in &report.results {
        if !result.passed {
            println!("FAIL  [{}/{}]", result.skill, result.scenario);
            for assertion in &result.assertions {
                if !assertion.passed {
                    println!("  ✗ {}", assertion.message);
                }
            }
        }
    }

    if report.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
```

---

## EvalReport Fields

```rust
pub struct EvalReport {
    pub results:  Vec<ScenarioResult>,
    pub total:    usize,
    pub passed:   usize,
    pub failed:   usize,
    pub duration: Duration,
}

pub struct ScenarioResult {
    pub skill:      String,
    pub scenario:   String,
    pub passed:     bool,
    pub assertions: Vec<AssertionResult>,
    pub duration:   Duration,
    pub error:      Option<String>,
}
```

---

## CI Integration

Add eval runs to your CI pipeline to catch skill regressions:

```yaml
# .github/workflows/ci.yml (excerpt)
- name: Run skill evaluations
  run: cargo test -p orka-eval --test eval_suite
```

Or as a standalone step with a binary target:

```yaml
- name: Run evals
  run: |
    cargo build -p orka-eval --release
    ./target/release/orka-eval evals/
```

---

## Writing Good Scenarios

- **One behaviour per scenario** — give each scenario a single, focused
  assertion set so failures are easy to diagnose.
- **Cover the error path** — add a scenario with `is_ok = false` for invalid
  inputs to confirm the skill fails gracefully.
- **Use `max_duration_ms`** — helps catch performance regressions early.
- **Name scenarios descriptively** — the name appears in CI output; prefer
  `"returns_empty_array_when_no_matches"` over `"test2"`.
