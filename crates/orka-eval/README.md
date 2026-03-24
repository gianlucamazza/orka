# Orka Eval Framework

Test framework per valutare l'efficacia delle skill di Orka.

## Panoramica

`orka-eval` fornisce un framework basato su file TOML per definire scenari di test che verificano il comportamento delle skill. Ogni file `.eval.toml` contiene uno o più scenari con input e aspettative.

## Struttura File

```toml
# Nome della skill da testare (opzionale, inferito dal filename)
skill = "web_search"

# Lista di scenari di test
[[scenarios]]
name = "search_returns_results"
description = "Web search should return relevant results"

# Input passati alla skill
[scenarios.input]
query = "Rust programming language"
limit = 5

# Aspettative sul risultato
[scenarios.expected]
is_ok = true                    # La skill deve riuscire
contains = ["Rust", "programming"]  # L'output deve contenere queste stringhe
max_duration_ms = 2000          # Tempo massimo di esecuzione
format = "json"                 # L'output deve essere JSON valido
```

## Campi Expectations

| Campo | Tipo | Descrizione |
|-------|------|-------------|
| `is_ok` | `bool` | Se `true`, la skill deve riuscire; se `false`, deve fallire |
| `contains` | `Vec<String>` | Sottstringhe che devono apparire nell'output |
| `not_contains` | `Vec<String>` | Sottstringhe che NON devono apparire nell'output |
| `format` | `String` | Formato atteso (attualmente solo `"json"` supportato) |
| `output_matches` | `String` | Regex pattern che l'output deve matchare |
| `max_duration_ms` | `u64` | Durata massima in millisecondi |

## Utilizzo CLI

```bash
# Esegui tutti i test in una directory
orka eval run evals/

# Esegui test per una skill specifica
orka eval run evals/ --skill web_search

# Esegui un file specifico
orka eval run evals/web_search.eval.toml

# Output JSON
orka eval run evals/ --json
```

## Esempi

### Test base di una skill

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

### Test con regex

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

### Test di fallimento atteso

```toml
skill = "file_read"

[[scenarios]]
name = "read_nonexistent_file"
description = "Should fail when file doesn't exist"

[scenarios.input]
path = "/nonexistent/path/file.txt"

[scenarios.expected]
is_ok = false
max_duration_ms = 500
```

### Test multipli nella stessa file

```toml
skill = "web_search"

[[scenarios]]
name = "search_with_limit"
description = "Search should respect limit parameter"

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

## Integrazione CI

Aggiungi nel tuo workflow CI:

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

1. **Nomi descrittivi**: Usa nomi di scenario che descrivono il comportamento testato
2. **Aspettative specifiche**: Usa `contains` e `output_matches` per verifiche precise
3. **Timeout ragionevoli**: Imposta `max_duration_ms` appropriati per il tipo di skill
4. **Test di fallimento**: Includi scenari che testano il comportamento di errore
5. **Mantieni aggiornati**: Aggiorna gli eval quando cambi il comportamento delle skill

## Architettura

Il framework è composto da:

- **scenario.rs**: Definizioni `Scenario`, `Expectations`, `EvalFile`
- **assertion.rs**: Logica di validazione `check_all()`
- **runner.rs**: Esecutore `EvalRunner` che carica file e runna scenari
- **report.rs**: Report strutturati `EvalReport` e `ScenarioResult`

## API Rust

```rust
use orka_eval::{EvalRunner, EvalReport};
use std::sync::Arc;
use orka_skills::SkillRegistry;

let registry: Arc<SkillRegistry> = // ...
let runner = EvalRunner::new(registry);

// Esegui directory
let report: EvalReport = runner.run_dir("evals/", None).await?;

// Stampa report
report.print_pretty();

// O serializza in JSON
let json = report.to_json();
```
