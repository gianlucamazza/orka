# Piano di Refactor: config.rs Modularizzazione

## Stato Attuale

- **File:** `crates/orka-core/src/config.rs`
- **Dimensione:** 2,712 righe
- **Struct principali:** 31+
- **Funzioni default:** ~70
- **Implementazioni:** ~40

## Obiettivo

Suddividere in moduli per dominio mantenendo **100% compatibilità API** (no breaking changes).

---

## Struttura Proposta

```
crates/orka-core/src/config/
├── mod.rs           # Re-exports e OrkaConfig struct
├── loader.rs        # Load functions e validazione
├── defaults.rs      # Funzioni default_* condivise
├── core.rs          # WorkspaceEntry, metodi OrkaConfig base
├── server.rs        # ServerConfig
├── logging.rs       # LoggingConfig
├── redis.rs         # RedisConfig
├── bus.rs           # BusConfig
├── adapters.rs      # Tutte le adapter configs
├── worker.rs        # WorkerConfig
├── memory.rs        # MemoryConfig
├── security/
│   ├── mod.rs       # Re-exports
│   ├── secrets.rs   # SecretConfig
│   ├── auth.rs      # AuthConfig, JwtAuthConfig, ApiKeyEntry
│   └── sandbox.rs   # SandboxConfig
├── plugins.rs       # PluginConfig, PluginCapabilities
├── skills.rs        # SoftSkillConfig
├── session.rs       # SessionConfig
├── queue.rs         # QueueConfig
├── audit.rs         # AuditConfig
├── observe.rs       # ObserveConfig
├── gateway.rs       # GatewayConfig
├── llm.rs           # LlmConfig, LlmProviderConfig
├── agent.rs         # AgentConfig, AgentDef, GraphDef, EdgeDef
├── tools.rs         # ToolsConfig
├── protocols/
│   ├── mod.rs       # Re-exports
│   ├── mcp.rs       # McpConfig e related
│   ├── a2a.rs       # A2aConfig
│   └── guardrails.rs # GuardrailsConfig
├── os.rs            # OsConfig, ClaudeCodeConfig, SudoConfig
├── knowledge.rs     # KnowledgeConfig e embedding
├── scheduler.rs     # SchedulerConfig
├── http.rs          # HttpClientConfig, WebhookConfig
├── prompts.rs       # PromptsConfig
├── experience.rs    # ExperienceConfig
└── web.rs           # WebConfig
```

---

## Fasi di Migrazione

### Fase 1: Setup Struttura (30 min)

Creare directory e file moduli vuoti:

```bash
mkdir -p crates/orka-core/src/config/security
mkdir -p crates/orka-core/src/config/protocols
touch crates/orka-core/src/config/mod.rs
touch crates/orka-core/src/config/loader.rs
touch crates/orka-core/src/config/defaults.rs
touch crates/orka-core/src/config/core.rs
touch crates/orka-core/src/config/server.rs
touch crates/orka-core/src/config/logging.rs
touch crates/orka-core/src/config/redis.rs
touch crates/orka-core/src/config/bus.rs
touch crates/orka-core/src/config/adapters.rs
touch crates/orka-core/src/config/worker.rs
touch crates/orka-core/src/config/memory.rs
touch crates/orka-core/src/config/security/mod.rs
touch crates/orka-core/src/config/security/secrets.rs
touch crates/orka-core/src/config/security/auth.rs
touch crates/orka-core/src/config/security/sandbox.rs
touch crates/orka-core/src/config/plugins.rs
touch crates/orka-core/src/config/skills.rs
touch crates/orka-core/src/config/session.rs
touch crates/orka-core/src/config/queue.rs
touch crates/orka-core/src/config/audit.rs
touch crates/orka-core/src/config/observe.rs
touch crates/orka-core/src/config/gateway.rs
touch crates/orka-core/src/config/llm.rs
touch crates/orka-core/src/config/agent.rs
touch crates/orka-core/src/config/tools.rs
touch crates/orka-core/src/config/protocols/mod.rs
touch crates/orka-core/src/config/protocols/mcp.rs
touch crates/orka-core/src/config/protocols/a2a.rs
touch crates/orka-core/src/config/protocols/guardrails.rs
touch crates/orka-core/src/config/os.rs
touch crates/orka-core/src/config/knowledge.rs
touch crates/orka-core/src/config/scheduler.rs
touch crates/orka-core/src/config/http.rs
touch crates/orka-core/src/config/prompts.rs
touch crates/orka-core/src/config/experience.rs
touch crates/orka-core/src/config/web.rs
```

### Fase 2: Migrazione Default Functions (20 min)

**File:** `config/defaults.rs`

```rust
//! Default value functions for configuration.

pub fn default_host() -> String {
    "127.0.0.1".into()
}

pub fn default_port() -> u16 {
    8080
}

pub fn default_workspace_dir() -> String {
    "./workspace".into()
}

pub fn default_bus_backend() -> String {
    "redis".into()
}

// ... tutte le altre funzioni default
```

### Fase 3: Migrazione per Modulo (3-4 ore)

Ordine consigliato (dipendenze minime prima):

1. `defaults.rs` - Nessuna dipendenza
2. `core.rs` - WorkspaceEntry, dipende da defaults
3. `server.rs` - Dipende da defaults
4. `logging.rs` - Dipende da defaults
5. `redis.rs` - Dipende da defaults
6. `bus.rs` - Dipende da defaults
7. `security/*.rs` - Auth, secrets, sandbox
8. `adapters.rs` - Dipende da security per auth types
9. `llm.rs` - Grande modulo, dipende da security
10. `agent.rs` - Dipende da llm
11. `memory.rs`, `session.rs`, `queue.rs`
12. `tools.rs`, `skills.rs`, `plugins.rs`
13. `protocols/*.rs` - MCP, A2A, guardrails
14. `os.rs`, `knowledge.rs`, `scheduler.rs`
15. `http.rs`, `web.rs`, `prompts.rs`, `experience.rs`
16. `observe.rs`, `audit.rs`, `gateway.rs`
17. `loader.rs` - Load e validate functions
18. `mod.rs` - OrkaConfig che aggrega tutto

### Fase 4: Re-export e Compatibilità (30 min)

**File:** `config/mod.rs`

```rust
//! Configuration module for Orka.
//!
//! This module provides type-safe configuration structures
//! with serde deserialization support.

// Re-export all configuration types for backward compatibility
pub use self::adapters::*;
pub use self::agent::*;
pub use self::audit::*;
pub use self::bus::*;
pub use self::core::*;
pub use self::experience::*;
pub use self::gateway::*;
pub use self::http::*;
pub use self::knowledge::*;
pub use self::loader::*;
pub use self::llm::*;
pub use self::logging::*;
pub use self::memory::*;
pub use self::observe::*;
pub use self::os::*;
pub use self::plugins::*;
pub use self::prompts::*;
pub use self::protocols::*;
pub use self::queue::*;
pub use self::redis::*;
pub use self::scheduler::*;
pub use self::security::*;
pub use self::server::*;
pub use self::session::*;
pub use self::skills::*;
pub use self::tools::*;
pub use self::web::*;
pub use self::worker::*;

pub mod adapters;
pub mod agent;
pub mod audit;
pub mod bus;
pub mod core;
pub mod defaults;
pub mod experience;
pub mod gateway;
pub mod http;
pub mod knowledge;
pub mod loader;
pub mod llm;
pub mod logging;
pub mod memory;
pub mod observe;
pub mod os;
pub mod plugins;
pub mod prompts;
pub mod protocols;
pub mod queue;
pub mod redis;
pub mod scheduler;
pub mod security;
pub mod server;
pub mod session;
pub mod skills;
pub mod tools;
pub mod web;
pub mod worker;
```

### Fase 5: Verifica (30 min)

```bash
# Verifica compilazione
cargo check --package orka-core

# Verifica test passano
cargo test --package orka-core

# Verifica nessun breaking change
cargo check --workspace
```

---

## Schema di Esempio: Migrazione Modulo Server

**Prima (in config.rs):**
```rust
/// HTTP server bind configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// IP address or hostname to bind on.
    #[serde(default = "default_host")]
    pub host: String,
    /// TCP port to listen on.
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    8080
}
```

**Dopo (config/server.rs):**
```rust
//! HTTP server configuration.

use serde::Deserialize;
use crate::config::defaults;

/// HTTP server bind configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// IP address or hostname to bind on.
    #[serde(default = "defaults::default_host")]
    pub host: String,
    /// TCP port to listen on.
    #[serde(default = "defaults::default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: defaults::default_host(),
            port: defaults::default_port(),
        }
    }
}
```

---

## Gestione Dipendenze Circolari

Il modulo `loader.rs` (validate/load) dipende da TUTTI gli altri moduli:

```rust
// config/loader.rs
use crate::config::*;

impl OrkaConfig {
    pub fn validate(&mut self) -> crate::Result<()> {
        // ... validation logic
    }
    
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        // ... load logic
    }
}
```

Soluzione: `loader.rs` è l'ultimo modulo da creare e può importare tutto da `config::*`.

---

## Checklist Prima del Merge

- [ ] `cargo check --package orka-core` passa
- [ ] `cargo test --package orka-core` passa
- [ ] `cargo clippy --package orka-core` nessun warning nuovo
- [ ] `cargo doc --package orka-core` documentazione completa
- [ ] Nessun uso di `super::` nel codice migliorato
- [ ] Tutti i `pub use` sono presenti in `config/mod.rs`
- [ ] File `config.rs` originale eliminato
- [ ] Benchmark prima/dopo (righe per file)

---

## Tempo Stimato

| Fase | Tempo |
|------|-------|
| Setup struttura | 30 min |
| Migrazione defaults | 20 min |
| Migrazione moduli (25 × 10 min) | ~4h |
| Re-export e compatibilità | 30 min |
| Verifica e fix | 30 min |
| **Totale** | **~6 ore** |

---

## Vantaggi Attesi

1. **Manutenibilità:** Ogni modulo < 200 righe
2. **Navigabilità:** Facile trovare configurazione per dominio
3. **Parallelizzazione:** Più sviluppatori possono lavorare su moduli diversi
4. **Testabilità:** Ogni modulo può avere i propri test
5. **Compilazione incrementale:** Modifica a un modulo non ricompila tutto
