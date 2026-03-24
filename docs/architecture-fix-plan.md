# Piano di Correzione Architetturale

## Analisi Root Cause

### 1. AsyncServiceContainer - Race Condition Fondamentale

**Problema:** L'implementazione attuale usa `remove()` sulla factory, causando race condition quando multipli task richiedono lo stesso servizio contemporaneamente.

**Codice problematico (riga 287-290):**
```rust
let factory_opt = {
    let mut factories = self.factories.lock().await;
    factories.remove(&type_id)  // Solo il primo task ottiene la factory
};
```

**Conseguenza:** I task successivi trovano `factory_opt = None` e falliscono, anche se il servizio non è ancora stato creato.

**Soluzione Architetturale Corretta:**

```rust
use tokio::sync::OnceCell;
use std::collections::HashMap;

pub struct AsyncServiceContainer {
    services: RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
    factories: RwLock<HashMap<TypeId, Box<dyn Fn() -> Pin<Box<dyn Future<Output = Arc<dyn Any + Send + Sync>> + Send>> + Send + Sync>>>,
    // Traccia quali servizi sono in fase di inizializzazione
    initializing: Mutex<HashMap<TypeId, Arc<tokio::sync::Notify>>>,
}

impl AsyncServiceContainer {
    pub async fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        
        // Fast path: già inizializzato
        {
            let services = self.services.read().await;
            if let Some(svc) = services.get(&type_id) {
                return svc.clone().downcast::<T>().ok();
            }
        }
        
        // Slow path: dobbiamo inizializzare o aspettare
        let notify = {
            let mut initializing = self.initializing.lock().await;
            
            // Se qualcuno sta già inizializzando, aspettiamo
            if let Some(notify) = initializing.get(&type_id) {
                Some(notify.clone())
            } else {
                // Siamo i primi, creiamo il notify
                let notify = Arc::new(tokio::sync::Notify::new());
                initializing.insert(type_id, notify.clone());
                None  // Indica che dobbiamo inizializzare noi
            }
        };
        
        if let Some(notify) = notify {
            // Aspettiamo che qualcun altro finisca
            notify.notified().await;
            
            // Ora dovrebbe essere disponibile
            let services = self.services.read().await;
            return services.get(&type_id)?.clone().downcast::<T>().ok();
        }
        
        // Inizializziamo noi
        let result = self.initialize_service(type_id).await;
        
        // Notifichiamo chi stava aspettando
        {
            let mut initializing = self.initializing.lock().await;
            if let Some(notify) = initializing.remove(&type_id) {
                notify.notify_waiters();
            }
        }
        
        result.downcast::<T>().ok()
    }
}
```

**Alternative Moderne:**
- Usare `async_once_cell::Lazy` per servizi singleton
- Considerare `moka` cache per servizi con TTL
- Valutare `diot` o `async DI` crate per dependency injection async

---

### 2. Cargo Deny - Configurazione Obsoleta

**Problema:** `cargo-deny 0.19.0` usa il formato di configurazione v2, ma il progetto ha ancora il formato v1.

**Cambiamenti Necessari (Best Practice):**

```toml
# deny.toml - Versione Moderna (v2)

[graph]
targets = [
    { triple = "x86_64-unknown-linux-gnu" },
    { triple = "x86_64-unknown-linux-musl" },
    { triple = "aarch64-unknown-linux-gnu" },
]

[advisories]
version = 2
yanked = "deny"

[licenses]
version = 2
allow = [
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "MIT",
    "MIT-0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Unlicense",
    "Zlib",
    "MPL-2.0",
    "CC0-1.0",
    "CDLA-Permissive-2.0",
    "LicenseRef-ring",
]

[[licenses.clarify]]
name = "ring"
expression = "LicenseRef-ring"
license-files = [{ path = "LICENSE", hash = 0xbd0eed23 }]

[[licenses.clarify]]
name = "webpki"
expression = "ISC"
license-files = [{ path = "LICENSE", hash = 0x001c7e6c }]

[bans]
multiple-versions = "warn"
wildcards = "allow"
highlight = "all"

# Gestione duplicati noti - approccio moderno
skip = [
    { name = "bitflags", version = "1.3.2" },
    { name = "hashbrown", version = "0.12.3" },
    { name = "indexmap", version = "1.9.3" },
    # ... altri
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

**Nota:** Alcuni warning su duplicati (es. `axum 0.7` vs `0.8`) sono dovuti a dipendenze transitive di `qdrant-client` e `opentelemetry-otlp`. Questi richiedono aggiornamento upstream.

---

### 3. Test orka-mcp - API Drift

**Problema:** Il tipo `McpTransportConfig::Stdio` ha aggiunto il campo `working_dir: Option<PathBuf>`, ma i test non sono stati aggiornati.

**Root Cause:** Mancanza di CI che fallisca su test rotti, o test ignorati.

**Soluzione Corretta:**

1. Aggiornare i test per usare struct update syntax:
```rust
McpTransportConfig::Stdio {
    command: "echo".into(),
    args: vec![],
    env: HashMap::new(),
    working_dir: None,  // nuovo campo
}
```

2. Implementare builder pattern per evitare breaking changes future:
```rust
impl McpTransportConfig {
    pub fn stdio(command: impl Into<String>) -> StdioBuilder {
        StdioBuilder {
            command: command.into(),
            args: vec![],
            env: HashMap::new(),
            working_dir: None,
        }
    }
}

pub struct StdioBuilder { ... }

impl StdioBuilder {
    pub fn args(mut self, args: Vec<String>) -> Self { ... }
    pub fn env(mut self, env: HashMap<String, String>) -> Self { ... }
    pub fn working_dir(mut self, dir: PathBuf) -> Self { ... }
    pub fn build(self) -> McpTransportConfig { ... }
}
```

---

### 4. Problema API Factory Type (Container)

**Problema:** Il bound `impl Fn() -> Pin<Box<dyn Future<Output = T> + Send>>` è difficile da usare correttamente perché richiede type annotations esplicite.

**Soluzione Moderna - async_trait o fn trait:**

```rust
// Approccio 1: Async trait (più pulito per l'utente)
#[async_trait::async_trait]
pub trait ServiceFactory<T: Send + Sync>: Send + Sync {
    async fn create(&self) -> T;
}

pub async fn register_async<T: Send + Sync + 'static>(
    &self,
    factory: impl ServiceFactory<T>,
) {
    // implementazione
}

// Approccio 2: Macro per registrazione ergonomica
#[macro_export]
macro_rules! register_async_service {
    ($container:expr, $ty:ty, || $body:expr) => {
        $container.register_async::<$ty>(Box::new(|| {
            Box::pin(async move { $body })
        })).await
    };
}
```

---

## Piano di Migrazione

### Fase 1: Fix Immediati (Blocker)
- [ ] Aggiornare deny.toml al formato v2
- [ ] Fix test orka-mcp aggiungendo `working_dir: None`
- [ ] Disabilitare test container concorrenziali fino a fix architetturale

### Fase 2: Refactoring Architetturale (1-2 settimane)
- [ ] Design review nuovo AsyncServiceContainer
- [ ] Implementazione con `tokio::sync::Notify` pattern
- [ ] Aggiungere test di stress per concorrenza
- [ ] Benchmark pre/post refactoring

### Fase 3: Modernizzazione API (2-3 settimane)
- [ ] Implementare builder pattern per McpTransportConfig
- [ ] Valutare crate DI esterni (`diot`, `shaku`, `azuretz`)
- [ ] Proposta RFC per nuova API container

### Fase 4: Prevenzione Regressioni
- [ ] Aggiungere `cargo test --workspace` come required check in CI
- [ ] Aggiungere `cargo deny check` in CI (allow-warnings)
- [ ] Policy: nessun merge con test rotti

---

## Risorse Consigliate

- **DI in Rust:** https://github.com/Alkass/diot
- **Async Initialization:** https://docs.rs/async-once-cell/
- **Cargo Deny v2:** https://embarkstudios.github.io/cargo-deny/checks/cfg.html
- **Builder Pattern:** https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
