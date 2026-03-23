# Best Practices Rust 2026 - Orka

Questo documento definisce le best practices per lo sviluppo di Orka nel 2026, sfruttando Rust 1.93+ e Edition 2024.

## 1. Pattern Moderni (Rust 1.93+)

### 1.1 `let else` per Early Return

Usare `let else` invece di `if let Some(...) {} else {}` per ridurre l'indentazione:

```rust
// ❌ Vecchio stile (molta indentazione)
if let Some(value) = option {
    // logica principale
} else {
    return Err(Error::NotFound);
}

// ✅ Moderno (flat, leggibile)
let Some(value) = option else {
    return Err(Error::NotFound);
};
// logica principale senza indentazione
```

### 1.2 `std::sync::LazyLock` al posto di `once_cell`/`lazy_static`

```rust
// ❌ Vecchio (dipendenze esterne)
use once_cell::sync::Lazy;
static CONFIG: Lazy<String> = Lazy::new(|| load_config());

// ✅ Moderno (standard library)
use std::sync::LazyLock;
static CONFIG: LazyLock<String> = LazyLock::new(|| load_config());
```

### 1.3 Async Closures

```rust
// ❌ Vecchio (async block esplicito)
let fut = async move { do_something().await };

// ✅ Moderno (async closure)
let f = async || do_something().await;
let result = f().await;

// Utile per callback
items.iter().map(async |item| {
    process(item).await
})
```

### 1.4 `if let` Concatenati (Rust 1.85+)

```rust
// ❌ Vecchio (annidato)
if let Some(outer) = option {
    if let Some(inner) = outer.field {
        // usa inner
    }
}

// ✅ Moderno (concatenato)
if let Some(outer) = option
    && let Some(inner) = outer.field
{
    // usa inner
}
```

### 1.5 `match` con Guardie

```rust
// ✅ Moderno (pattern matching espressivo)
match result {
    Ok(val) if val > 0 => println!("positivo"),
    Ok(_) => println!("zero"),
    Err(e) => println!("errore: {e}"),
}
```

## 2. Gestione Errori

### 2.1 `?` con Conversioni Automatiche

```rust
// ✅ Usare anyhow/thiserror per errori ricchi
use anyhow::Context;

let data = std::fs::read(path)
    .with_context(|| format!("failed to read {path:?}"))?;
```

### 2.2 `Result` in Collectioni

```rust
// ✅ Collect trasforma Vec<Result<T, E>> in Result<Vec<T>, E>
let results: Result<Vec<_>> = items
    .iter()
    .map(|item| process(item))
    .collect();
```

## 3. Async/Await Patterns

### 3.1 Native Async Traits

```rust
// ✅ Native async (no boxing, zero-cost)
pub trait MyTrait {
    async fn method(&self) -> Result<T>;
}

// Per trait object usare `async_trait` o:
pub trait MyTrait {
    fn method(&self) -> impl Future<Output = Result<T>> + Send;
}
```

### 3.2 `tokio::select!` con Biased

```rust
// ✅ Priorità esplicita
select! {
    biased; // Preferisci il primo branch
    
    _ = shutdown.cancelled() => {
        println!("shutting down");
    }
    msg = receiver.recv() => {
        process(msg).await;
    }
}
```

### 3.3 Cancellation Tokens

```rust
// ✅ Gestione graceful shutdown
use tokio_util::sync::CancellationToken;

let token = CancellationToken::new();
let child = token.child_token();

tokio::spawn(async move {
    select! {
        _ = child.cancelled() => {}
        _ = work() => {}
    }
});

token.cancel(); // Cancella tutti i child
```

## 4. Ottimizzazioni

### 4.1 `String` vs `&str`

```rust
// ✅ Usare `&str` per parametri di sola lettura
fn process(data: &str) -> Result<&str> { ... }

// ✅ `String` solo quando necessario ownership
fn store(data: String) { ... }

// ✅ `Cow<str>` per flessibilità
use std::borrow::Cow;

fn maybe_process(data: Cow<'_, str>) -> Cow<'_, str> {
    if needs_processing(&data) {
        Cow::Owned(process_owned(data.into_owned()))
    } else {
        data // no allocazione
    }
}
```

### 4.2 Iteratori Lazy

```rust
// ❌ Allocazione intermedia
let filtered: Vec<_> = items.iter().filter(|x| x > 0).collect();
let sum: i32 = filtered.iter().sum();

// ✅ Catena lazy, nessuna allocazione
let sum: i32 = items.iter().filter(|x| x > 0).sum();
```

### 4.3 `pin!` Macro

```rust
// ❌ Boxing sul heap
let fut = Box::pin(async { ... });

// ✅ Pinning sullo stack (no allocazione)
use std::pin::pin;
let fut = pin!(async { ... });
```

## 5. Testing

### 5.1 Test Parametrici

```rust
#[test_case(2, 4)]
#[test_case(3, 9)]
#[test_case(4, 16)]
fn test_square(input: i32, expected: i32) {
    assert_eq!(input * input, expected);
}
```

### 5.2 Async Tests

```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_concurrent() {
    let (tx, rx) = tokio::sync::mpsc::channel(10);
    
    tokio::spawn(async move {
        for i in 0..100 {
            tx.send(i).await.unwrap();
        }
    });
    
    let sum: i32 = rx.take(100).sum().await;
    assert_eq!(sum, 4950);
}
```

### 5.3 Property-Based Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_serialize_roundtrip(envelope in envelope_strategy()) {
        let json = serde_json::to_string(&envelope).unwrap();
        let restored: Envelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope.id, restored.id);
    }
}
```

## 6. Documentazione

### 6.1 Doc Tests Eseguibili

```rust
/// Calcola la somma di due numeri.
///
/// # Examples
///
/// ```
/// use orka_core::math;
/// assert_eq!(math::add(2, 3), 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

### 6.2 Documentazione API Completa

```rust
/// Errors that can occur during message processing.
///
/// This enum covers all recoverable and non-recoverable errors
/// that the message bus might encounter.
#[derive(Debug, Error)]
#[non_exhaustive] // Per permettere nuovi varianti in futuro
pub enum BusError {
    /// The connection to the message broker was lost.
    #[error("connection lost: {0}")]
    ConnectionLost(String),
    
    /// A message failed serialization.
    #[error("serialization failed")]
    Serialization(#[from] serde_json::Error),
}
```

## 7. Sicurezza

### 7.1 Zeroize per Secrets

```rust
use zeroize::Zeroize;

#[derive(Zeroize)]
#[zeroize(drop)] // Cancella memoria al drop
pub struct SecretKey([u8; 32]);
```

### 7.2 Sanitizzazione Input

```rust
/// Sanitizza input utente per prevenire injection.
pub fn sanitize(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .take(MAX_INPUT_LEN)
        .collect()
}
```

## 8. Strumenti Consigliati

### 8.1 Cargo Plugins

```bash
# Test avanzati
cargo install cargo-nextest

# Controllo dipendenze obsolete
cargo install cargo-udeps

# Controllo licenze
cargo install cargo-deny

# Formattazione
cargo install cargo-fmt

# Linting aggiuntivo
cargo install cargo-clippy
```

### 8.2 CI/CD

```yaml
# .github/workflows/ci.yml
- name: Clippy
  run: cargo clippy --all-targets --all-features -- -D warnings

- name: Test
  run: cargo nextest run --workspace

- name: Doc
  run: cargo doc --no-deps --document-private-items
```

## 9. Anti-Patterns da Evitare

### 9.1 `unwrap()` in Produzione

```rust
// ❌ PANIC in produzione
let value = some_option.unwrap();

// ✅ Gestione esplicita
let Some(value) = some_option else {
    return Err(Error::MissingValue);
};
```

### 9.2 `clone()` Non Necessario

```rust
// ❌ Clone implicito in loop
for item in &items {
    process(item.clone()); // item già &T
}

// ✅ Borrowing corretto
for item in &items {
    process(item); // &T è sufficiente
}
```

### 9.3 `Mutex` in Async

```rust
// ❌ Bloccare il runtime
let data = mutex.lock().unwrap();

// ✅ Async mutex
let data = async_mutex.lock().await;
```

## 10. Convenzioni di Nome

| Tipo | Convenzione | Esempio |
|------|-------------|---------|
| Struct/Enum | PascalCase | `MessageBus` |
| Funzioni/Metodi | snake_case | `send_message` |
| Costanti | SCREAMING_SNAKE_CASE | `MAX_RETRY` |
| Trait | PascalCase (descrittivo) | `Readable` |
| Type Aliases | PascalCase | `UserId` |
| Lifetime | minuscola breve | `'a`, `'ctx` |
| Generic | PascalCase singola lettera | `T`, `E` |

---

*Ultimo aggiornamento: Marzo 2026*
*Rust Version: 1.93.1*
*Edition: 2024*
