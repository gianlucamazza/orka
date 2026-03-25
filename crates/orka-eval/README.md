# Orka Eval Framework

Test framework for evaluating the effectiveness of Orka skills.

## Overview

`orka-eval` provides a TOML-based framework for defining test scenarios that verify skill behavior. Each `.eval.toml` file contains one or more scenarios with inputs and expectations.

## File Structure

```toml
# Skill to test (optional, inferred from filename)
skill = "web_search"

# List of test scenarios
[[scenarios]]
name = "search_returns_results"
description = "Web search should return relevant results"

# Input passed to the skill
[scenarios.input]
query = "Rust programming language"
limit = 5

# Expectations on the result
[scenarios.expected]
is_ok = true                    # The skill must succeed
contains = ["Rust", "programming"]  # Output must contain these strings
max_duration_ms = 2000          # Maximum execution time
format = "json"                 # Output must be valid JSON
```

## Expectation Fields

| Field | Type | Description |
|-------|------|-------------|
| `is_ok` | `bool` | If `true`, the skill must succeed; if `false`, it must fail |
| `contains` | `Vec<String>` | Substrings that must appear in the output |
| `not_contains` | `Vec<String>` | Substrings that must NOT appear in the output |
| `format` | `String` | Expected format (currently only `"json"` is supported) |
| `output_matches` | `String` | Regex pattern the output must match |
| `max_duration_ms` | `u64` | Maximum duration in milliseconds |

## CLI Usage

```bash
# Run all tests in a directory
orka eval run evals/

# Run tests for a specific skill
orka eval run evals/ --skill web_search

# Run a specific file
orka eval run evals/web_search.eval.toml

# JSON output
orka eval run evals/ --json
```

## Examples

### Basic skill test

```toml
skill = "file_read"

[[scenarios]]
name = "read_existing_file"
description = "Should read an existing file"

[scenarios.input]
path = "/etc/hosts"

[scenarios.expected]
is_ok = true
max_duration_ms = 500
```

### Test with regex

```toml
skill = "shell"

[[scenarios]]
name = "date_format"
description = "Date command should output expected format"

[scenarios.input]
command = "date +%Y-%m-%d"

[scenarios.expected]
is_ok = true
output_matches = "\\d{4}-\\d{2}-\\d{2}"
max_duration_ms = 1000
```

### Expected failure test

```toml
skill = "file_read"

[[scenarios]]
name = "read_nonexistent_file"
description = "Should fail when file does not exist"

[scenarios.input]
path = "/nonexistent/path/file.txt"

[scenarios.expected]
is_ok = false
max_duration_ms = 500
```

### Multiple scenarios in one file

```toml
skill = "web_search"

[[scenarios]]
name = "search_with_limit"
description = "Search should respect the limit parameter"

[scenarios.input]
query = "AI"
limit = 3

[scenarios.expected]
is_ok = true
max_duration_ms = 3000

[[scenarios]]
name = "search_empty_query"
description = "Empty query should fail gracefully"

[scenarios.input]
query = ""

[scenarios.expected]
is_ok = false
```

## CI Integration

Add to your CI workflow:

```yaml
- name: Run skill evaluations
  run: orka eval run evals/ --json > eval-results.json

- name: Check eval results
  run: |
    PASSED=$(jq '.passed' eval-results.json)
    TOTAL=$(jq '.total' eval-results.json)
    if [ "$PASSED" != "$TOTAL" ]; then
      echo "Some evals failed: $PASSED/$TOTAL"
      exit 1
    fi
```

## Best Practices

1. **Descriptive names**: Use scenario names that describe the behavior being tested
2. **Specific expectations**: Use `contains` and `output_matches` for precise checks
3. **Reasonable timeouts**: Set `max_duration_ms` appropriate to the skill type
4. **Failure tests**: Include scenarios that test error behavior
5. **Keep up to date**: Update evals when changing skill behavior

## Architecture

The framework consists of:

- **scenario.rs**: `Scenario`, `Expectations`, `EvalFile` definitions
- **assertion.rs**: Validation logic `check_all()`
- **runner.rs**: `EvalRunner` that loads files and runs scenarios
- **report.rs**: Structured reports `EvalReport` and `ScenarioResult`

## Rust API

```rust
use orka_eval::{EvalRunner, EvalReport};
use std::sync::Arc;
use orka_skills::SkillRegistry;

let registry: Arc<SkillRegistry> = // ...
let runner = EvalRunner::new(registry);

// Run a directory
let report: EvalReport = runner.run_dir("evals/", None).await?;

// Print report
report.print_pretty();

// Or serialize to JSON
let json = report.to_json();
```
