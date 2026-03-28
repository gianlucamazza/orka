# Orka — Analisi Architettura & Design

**Data:** 2026-03-25
**Versione:** 1.0.0
**Branch:** main (18 commit avanti a origin/main)
**Crate totali nel workspace:** 40 (38 in `crates/` + 3 in `sdk/`)
**Baseline precedente:** `docs/internal/analysis-report.md` del 2026-03-23

> **⚠️ Nota storica (2026-03-28):** Questo documento è uno snapshot del 2026-03-25. Da allora:
> - `orka-core/src/container.rs` è stato **rimosso** (§3 e §4 sono riferimenti storici).
> - I moduli di config in `orka-core/src/config/` sono stati ridotti a 3 (`agent`, `defaults`, `primitives`); la config di dominio è migrata nei crate proprietari, quella runtime top-level in `orka-config/src/runtime.rs`.

---

## 1. Grafo Dipendenze Inter-Crate

### Gerarchia dichiarata nel workspace

Il `Cargo.toml` radice definisce 8 layer logici (dal più basso al più alto):

```
Layer 0  – Core          : orka-core, orka-circuit-breaker
Layer 1  – Infrastructure: orka-bus, orka-queue, orka-session, orka-memory,
                           orka-secrets, orka-scheduler, orka-observe
Layer 2  – AI/Intelligence: orka-llm, orka-knowledge, orka-prompts,
                            orka-guardrails, orka-experience, orka-eval,
                            orka-research  ← NEW
Layer 3  – Execution/Ext: orka-skills, orka-wasm, orka-sandbox,
                          orka-mcp, orka-a2a  ← NEW
Layer 4  – Adapters:      orka-adapter-telegram/discord/whatsapp/slack/custom
Layer 5  – Dev Tools:     orka-git
Layer 6  – Networking/OS: orka-http, orka-web, orka-auth, orka-os
Layer 7  – Orchestration: orka-checkpoint, orka-agent, orka-workspace,
                          orka-gateway, orka-worker, orka-cli, orka-server
Layer 8  – Plugin SDK:    orka-plugin-sdk, orka-plugin-sdk-component,
                          hello-plugin
```

### Mappa completa delle dipendenze (solo crate interni)

| Crate | Dipende da (crate interni) |
|---|---|
| `orka-core` | *(nessuna)* |
| `orka-circuit-breaker` | *(nessuna)* |
| `orka-bus` | core |
| `orka-queue` | core |
| `orka-session` | core |
| `orka-memory` | core |
| `orka-secrets` | core |
| `orka-scheduler` | core |
| `orka-observe` | core |
| `orka-auth` | core |
| `orka-http` | core |
| `orka-web` | core |
| `orka-os` | core |
| `orka-git` | core |
| `orka-knowledge` | core |
| `orka-prompts` | core |
| `orka-wasm` | core |
| `orka-llm` | core, circuit-breaker |
| `orka-guardrails` | core, llm |
| `orka-experience` | core, knowledge, llm, prompts |
| `orka-eval` | core, llm, skills |
| `orka-research` *(NEW)* | core, **checkpoint**, skills, scheduler |
| `orka-skills` | core, wasm, circuit-breaker, prompts |
| `orka-sandbox` | core, wasm |
| `orka-mcp` | core, skills |
| `orka-a2a` *(NEW)* | core, skills |
| `orka-adapter-telegram` | core |
| `orka-adapter-discord` | core |
| `orka-adapter-whatsapp` | core |
| `orka-adapter-slack` | core |
| `orka-adapter-custom` | core, auth |
| `orka-workspace` | core, prompts |
| `orka-checkpoint` | core, **llm** |
| `orka-agent` | core, llm, skills, experience, workspace, guardrails, prompts, checkpoint |
| `orka-gateway` | core, bus, session, queue, workspace |
| `orka-worker` | core, queue, session, bus, skills, workspace, llm, guardrails, experience, agent, prompts |
| `orka-cli` | core, mcp, secrets, skills, wasm |
| `orka-server` | tutti i crate sopra, inclusi a2a e research |

### Cicli

**Nessun ciclo rilevato.** Il grafo è un DAG (grafo aciclico diretto).

Verifica delle catene più lunghe:
- `server → worker → agent → experience → knowledge → core` ✅
- `server → research → checkpoint → llm → core` ✅  
- Nessuna catena torna a un nodo già visitato.

### Verifica orka-core come leaf

`orka-core/Cargo.toml` non dichiara **nessuna** dipendenza verso altri crate `orka-*`.  
**orka-core è e rimane un leaf puro.** ✅

---

## 2. Violazioni di Layering

### V-1 — `orka-research` (Layer 2 AI) → `orka-checkpoint` (Layer 7 Orchestration) 🔴 P1

`orka-research` è classificato nel workspace come crate di "AI/Intelligence" (Layer 2), ma dipende da `orka-checkpoint` che è "Orchestration" (Layer 7). Un crate di livello inferiore che tira dentro uno di livello superiore è la definizione di violazione di layering.

**Path concreto:**
```
orka-research (AI, L2) ──depends-on──► orka-checkpoint (Orchestration, L7)
```

**Motivazione apparente:** `ResearchService` usa `CheckpointStore` per persistere lo stato delle campaign run come se fossero execution checkpoint.  
**Fix suggerito (senza modificare file):** Spostare `orka-research` nel Layer 7 (Orchestration), oppure estrarre il trait `CheckpointStore` in `orka-core` (che già espone trait d'infrastruttura), eliminando la dipendenza diretta da `orka-checkpoint`.

### V-2 — `orka-checkpoint` (Layer 7 Orchestration) → `orka-llm` (Layer 2 AI) 🟡 P2

`orka-checkpoint` contiene "checkpointing e crash recovery dei graph" ma dipende da `orka-llm`. Un crate di persistenza/recovery non dovrebbe conoscere il livello AI.

**Path concreto:**
```
orka-checkpoint (Orchestration, L7) ──depends-on──► orka-llm (AI, L2)
```

Dato che L7 dipende da L2, la direzione *sintattica* è corretta (alto→basso), ma semanticamente un checkpoint store che conosce il modello LLM viola il principio di separazione delle responsabilità. Probabilmente `orka-llm` è usato per il tipo `LlmMessage` che dovrebbe vivere in `orka-core`.

### V-3 — `orka-eval` (Layer 2 AI) → `orka-skills` (Layer 3 Execution) 🟡 P2

`orka-eval` è in AI/Intelligence ma dipende da `orka-skills` (Execution/Extensibility, Layer 3 — livello superiore nella gerarchia dichiarata).

```
orka-eval (AI, L2) ──depends-on──► orka-skills (Execution, L3)
```

Questo è un salto in avanti di un layer; per un crate di valutazione che esegue skill è comprensibile, ma non è allineato alla struttura dichiarata nel workspace.

### Tabella riassuntiva violazioni

| ID | Crate sorgente | Crate target | Severità | Fix |
|---|---|---|---|---|
| V-1 | `orka-research` (L2) | `orka-checkpoint` (L7) | 🔴 P1 | Spostare research in L7 o portare trait in core |
| V-2 | `orka-checkpoint` (L7) | `orka-llm` (L2) | 🟡 P2 | Spostare i tipi LLM condivisi in core |
| V-3 | `orka-eval` (L2) | `orka-skills` (L3) | 🟡 P2 | Spostare eval in L3 o usare trait da core |

---

## 3. Moduli di Grande Dimensione (> 500 righe)

Rilevati **34 file** con più di 500 righe (escludendo file sotto `tests/`). I test file sono annotati separatamente.

### File sorgente (non-test) > 500 righe

| # | File | Righe | Note |
|---|---|---|---|
| 1 | `orka-agent/src/node_runner.rs` | **1 717** | 🔴 God module — runner del nodo del grafo, candidato a split |
| 2 | `orka-core/src/migrate.rs` | **1 674** | 🟡 Logica migrazione config — strutturalmente coeso |
| 3 | `orka-worker/src/workspace_handler/mod.rs` | **1 614** | 🔴 Già segnalato nel report precedente, nessun miglioramento |
| 4 | `orka-cli/src/cmd/chat.rs` | **1 600** | 🔴 TUI chat monolitica |
| 5 | `orka-core/src/types.rs` | **1 385** | 🟡 Solo tipi/dati — accettabile ma monitorare |
| 6 | `orka-research/src/service.rs` *(NEW)* | **1 235** | 🔴 Nuovo crate, già god class al lancio |
| 7 | `orka-os/src/skills/fs.rs` | **1 086** | 🟡 Aggregazione skill FS — strutturalmente coeso |
| 8 | `orka-server/src/bootstrap.rs` | **987** | 🟡 Composition root — accettabile ma al limite |
| 9 | `orka-llm/src/anthropic.rs` | **953** | 🟡 Provider LLM specifico |
| 10 | `orka-cli/src/main.rs` | **873** | 🟡 Entry point CLI con routing comandi |
| 11 | `orka-agent/src/executor.rs` | **855** | 🟡 Graph executor |
| 12 | `orka-llm/src/openai.rs` | **836** | 🟡 Provider LLM specifico |
| 13 | `orka-cli/src/markdown.rs` | **814** | 🟡 Renderer Markdown |
| 14 | `orka-a2a/src/routes.rs` *(NEW)* | **779** | 🟡 Route handlers A2A — borderline |
| 15 | `orka-llm/src/client.rs` | **768** | 🟡 Aggregatore client LLM |
| 16 | `orka-os/src/skills/coding_delegate.rs` | **729** | 🟡 Skill delegation |
| 17 | `orka-core/src/container.rs` | **720** | 🟢 ~260 righe effettive, ~460 test inline |
| 18 | `orka-cli/src/cmd/dashboard.rs` | **706** | 🟡 TUI dashboard |
| 19 | `orka-os/src/skills/package.rs` | **698** | 🟡 Skill package manager |
| 20 | `orka-research/src/store.rs` *(NEW)* | **658** | 🟡 Doppio backend (in-memory + Redis) |
| 21 | `orka-core/src/testing.rs` | **647** | 🟢 Solo test doubles |
| 22 | `orka-cli/src/completion.rs` | **643** | 🟡 Shell completion — strutturalmente coeso |
| 23 | `orka-agent/src/config.rs` | **640** | 🟡 Config strutture |
| 24 | `orka-prompts/src/pipeline/builder.rs` | **621** | 🟡 Builder fluente — accettabile |
| 25 | `orka-a2a/src/types.rs` *(NEW)* | **602** | 🟢 Solo tipi di protocollo |
| 26 | `orka-adapter-slack/src/lib.rs` | **573** | 🟡 Adapter monolitico |
| 27 | `orka-observe/src/otel_sink.rs` | **565** | 🟡 OTel sink |
| 28 | `orka-agent/src/context.rs` | **565** | 🟡 Context del nodo |
| 29 | `orka-adapter-whatsapp/src/lib.rs` | **561** | 🟡 Adapter monolitico |
| 30 | `orka-adapter-discord/src/lib.rs` | **556** | 🟡 Adapter monolitico |
| 31 | `orka-observe/src/lib.rs` | **535** | 🟡 Aggregatore osservabilità |
| 32 | `orka-cli/src/shell.rs` | **522** | 🟡 Shell interattiva |
| 33 | `orka-llm/src/context.rs` | **502** | 🟢 Context window management |
| 34 | `orka-circuit-breaker/src/lib.rs` | **502** | 🟢 Self-contained, tutto il codice è nel file unico |

**File di test > 500 righe** (non contano come violazione ma indicano mancanza di fixtures):

| File | Righe |
|---|---|
| `orka-agent/tests/graph_executor_test.rs` | 1 164 |
| `orka-a2a/tests/a2a_test.rs` *(NEW)* | 777 |

**Riepilogo delta rispetto alla baseline (2026-03-23):**  
La baseline citava 12 file > 500 righe. Il conteggio attuale è **34** (fonte: inclusi tutti i file non-test).  
I 4 nuovi file aggiunti dai crate `orka-research` e `orka-a2a` contribuiscono **3 272 righe** aggregate.

---

## 4. ServiceContainer — Analisi Concorrenza

Fonte: `orka-core/src/container.rs` (720 righe, di cui ~460 test inline).

Sono presenti tre varianti:

### 4.1 `ServiceContainer` (sincrono)

```rust
pub struct ServiceContainer {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}
```

- Richiede `&mut self` per `register()` — nessuna condivisione durante la scrittura.
- `get()` richiede solo `&self` e ritorna un `Arc<T>` clonato — thread-safe per le letture.
- **Nessun lock, nessuna race condition possibile.**
- Pattern d'uso: inizializzazione single-threaded in bootstrap, poi condivisione via `Arc<ServiceContainer>`.

### 4.2 `LazyContainer` (sincrono lazy)

```rust
pub struct LazyContainer {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    factories: HashMap<TypeId, Box<dyn Fn() -> Arc<dyn Any + Send + Sync> + Send + Sync>>,
}
```

- Anche `get()` richiede `&mut self` (rimuove la factory da `factories` e inserisce in `services`).
- Strutturalmente single-threaded — nessuna race condition.
- **Limitazione**: non può essere condiviso tra task async senza wrapping in `Mutex`.

### 4.3 `AsyncServiceContainer` (async lazy, thread-safe)

```rust
pub struct AsyncServiceContainer {
    services: tokio::sync::RwLock<HashMap<TypeId, AsyncServiceEntry>>,
}
```

Questo è il componente più complesso. Analisi dettagliata:

#### Pattern double-check con Notify — Corretto ✅

Il codice implementa correttamente il pattern *double-checked locking* adattato all'async:

```
Fast path  (read lock)  → Initialized?  → return
                        → Initializing? → subscribe to Notify BEFORE drop(lock) → await → return
                        → Pending/None  → fall through
Slow path (write lock)  → double-check all states (same as above)
                        → if Pending: take factory, insert Initializing{notify}
Run factory (no lock held)
Write lock              → insert Initialized(instance)
notify.notify_waiters()
Read lock               → return value
```

L'iscrizione al `Notify` (`let notified = n.notified()`) avviene **prima** del `drop(services)` — questo è critico per evitare il *lost-wakeup*. Il commento nel codice lo documenta esplicitamente. ✅

Il lint `await_holding_lock = "deny"` (workspace) **non si attiva** perché il lock viene rilasciato con `drop(services)` prima di ogni `.await`. ✅

#### Potenziale deadlock per panic della factory 🔴 P1

```
Stato: Pending → write lock → rimuove factory → inserisce Initializing{notify}
       → drop write lock → factory().await  ← PANIC QUI
       → lo stato rimane Initializing per sempre
       → tutti i task successivi che chiamano get() attendono notify_waiters()
       → notify_waiters() non viene mai chiamato
       → DEADLOCK PERMANENTE per quel TypeId
```

**Scenario:** Se la factory asincrona fa `panic!` (o qualsiasi `panic` non catturato nel task), l'entry resta bloccata nello stato `Initializing`. Non c'è cleanup né timeout. Questo è un problema in produzione se si usa `register_async` con factory che possono fallire.

**Fix suggerito:** Wrappare la chiamata factory in `catch_unwind` o usare un `scopeguard` che in caso di errore reinserisce lo stato `Pending` o `Error`.

#### Nessun lock annidato ✅

Tutte le acquisizioni di lock in `AsyncServiceContainer::get()` sono sequenziali, non annidate. La sequenza è:
1. `read().await` (rilasciato con `drop`)
2. `write().await` (rilasciato con `drop`)
3. *(factory senza lock)*
4. `write().await` (rilasciato con `drop`)
5. `notify_waiters()`
6. `read().await` (rilasciato con `drop`)

Nessuna acquisizione di `write` all'interno di un `read` attivo, né viceversa. ✅

#### Granularità lock globale 🟡 P2

C'è un unico `RwLock` per l'intera mappa. In fase di inizializzazione parallela di molti servizi, tutti i task contendono sullo stesso lock. Per un container di servizi a uso bootstrap questo è accettabile; lo diventerebbe problematico se usato come cache per-request.

---

## 5. Analisi `orka-research`

### 5.1 Dipendenze

```
orka-research
  ├── orka-core          (L0 Core)        ✅ corretto
  ├── orka-checkpoint    (L7 Orchestration) ❌ VIOLAZIONE V-1
  ├── orka-skills        (L3 Execution)   ⚠️  salto di layer (L2→L3)
  └── orka-scheduler     (L1 Infrastructure) ✅ corretto
```

### 5.2 Posizionamento nel workspace

Il workspace dichiara `orka-research` in "AI/Intelligence" (`# LLM client, RAG, prompt engine, guardrails, self-learning, evaluation`). Il crate però implementa:
- Campaign management e run lifecycle → Orchestration concern
- Persistence Redis-backed → Infrastructure concern
- Skill exposure → Execution concern

La denominazione e il layer sono sbagliati. Funzionalmente `orka-research` è un **Orchestration** crate che coordina skill, checkpoint e scheduler. **Dovrebbe essere spostato in Layer 7**.

### 5.3 Temporal coupling via `OnceLock` 🟡 P2

```rust
pub struct ResearchService {
    skills: OnceLock<Arc<SkillRegistry>>,
    ...
}

pub fn bind_registry(&self, skills: Arc<SkillRegistry>) {
    let _ = self.skills.set(skills);
}
```

Il servizio è parzialmente inizializzato fino a quando `bind_registry()` non viene chiamato dal bootstrap. Ogni metodo che usa `self.registry()?` fallisce prima della chiamata. Questo è un anti-pattern (oggetti *zombie*) che può causare errori runtime difficili da diagnosticare se il bootstrap viene riorganizzato.

**Root cause:** Dipendenza circolare di inizializzazione:
```
ResearchService → (usa) → SkillRegistry
SkillRegistry   → (contiene) → ExperimentRunSkill
ExperimentRunSkill → (wrappa) → Arc<ResearchService>
```

**Fix:** Spezzare il ciclo spostando l'invocazione delle skill da `ResearchService` a un `ResearchOrchestrator` separato che conosce sia il servizio che il registry.

### 5.4 God class `service.rs` 🔴 P1

Con **1 235 righe** alla prima release, `service.rs` è già a rischio di diventare un god module. Contiene:
- CRUD per campaign, run, candidate, promotion request (4 domini)
- Logica di validazione
- Invocazione skill
- Logica scheduling
- Event emission

**Ogni dominio** (campaign, run, candidate, promotion) meriterebbe un modulo separato sotto `service/`.

### 5.5 Interfaccia coerente ✅

Le funzioni di fabbrica pubbliche (`create_research_store`, `create_research_service`, `create_research_skills`) seguono la convenzione degli altri crate. Il trait `ResearchStore` con backends `InMemoryResearchStore` / `RedisResearchStore` segue il pattern stabilito (es. `TaskStore` in a2a, `ScheduleStore` in scheduler). ✅

### 5.6 `unwrap`/`expect` nel crate

13 occorrenze di `.unwrap()` / `.expect()` in `service.rs`. Non eclatante ma da azzerare data la policy workspace (`unwrap_used = "warn"`).

---

## 6. Analisi `orka-a2a`

### 6.1 Dipendenze

```
orka-a2a
  ├── orka-core    (L0 Core)       ✅ corretto
  └── orka-skills  (L3 Execution)  ✅ stesso layer — accettabile
```

**Nessuna violazione di layering.** Il crate è coerentemente posizionato nell'Execution layer. ✅

### 6.2 Pattern di lock in `routes.rs`

`A2aState` contiene:

```rust
pub task_events: Arc<Mutex<HashMap<String, broadcast::Sender<Arc<TaskEvent>>>>>
```

Ogni handler acquisisce il lock in modo puntuale (insert, remove, get) senza tenere il lock attraverso `.await`. Esempio:

```rust
state.task_events.lock().await.insert(task_id.clone(), tx.clone());
// lock rilasciato qui — nessuna operazione async con lock held
```

**Nessun deadlock né violazione di `await_holding_lock`** nel codice esaminato. ✅

#### Race condition documentata e accettabile ⚠️ P3

In `handle_push_notification_set`:
1. `push_store.set(config).await?` — lock non tenuto
2. `task_events.lock().await.get(&task_id)` — lock acquisito dopo

Tra 1 e 2, il task potrebbe completare e la sua entry in `task_events` essere rimossa. Il caso è gestito dal fallback che controlla il `task_store`:

```rust
} else if let Ok(Some(task)) = state.task_store.get(&task_id).await {
    // Task already completed — deliver the terminal status once.
```

La race condition esiste ma il codice la gestisce correttamente. ✅

#### Scalabilità del `Mutex` globale su `task_events` 🟡 P2

Tutti i task (submitter, SSE stream, push notification) contendono su un unico `Mutex<HashMap>`. In scenari ad alta concorrenza (molti task paralleli) questo diventa un bottleneck. Un `DashMap<String, broadcast::Sender<...>>` eliminerebbe la contesa.

### 6.3 Interfaccia coerente ✅

`orka-a2a` espone un `a2a_router(state: A2aState) -> Router` che si monta in `orka-server/src/bootstrap.rs`. Il pattern è identico a quello usato da `orka-gateway` e `orka-observe`. ✅

Il doppio backend `InMemoryTaskStore` / `RedisTaskStore` segue la convenzione del workspace. ✅

### 6.4 Dimensioni file

- `routes.rs` (779 righe): limite superiore accettabile. Potrebbe essere diviso in `handlers/submit.rs`, `handlers/stream.rs`, `handlers/push.rs` per leggibilità.
- `types.rs` (602 righe): puramente tipi di protocollo — OK.
- `a2a_test.rs` (777 righe test): indicatore che il crate ha buona copertura di test al lancio. ✅

---

## 7. Findings Riassuntivi

### 🔴 P0 — Critico (blocco produzione)

*Nessun finding P0 rilevato.*

---

### 🔴 P1 — Alta Priorità

| ID | Area | Descrizione | Crate coinvolto |
|---|---|---|---|
| **P1-1** | Layering | `orka-research` (AI/Intelligence L2) dipende da `orka-checkpoint` (Orchestration L7): dipendenza upward che inverte la gerarchia dichiarata | `orka-research` |
| **P1-2** | Concorrenza | `AsyncServiceContainer`: se la factory asincrona fa panic, l'entry rimane in stato `Initializing` permanente → tutti i successivi `get()` bloccano su `notify_waiters()` che non viene mai chiamato (deadlock silente) | `orka-core` |
| **P1-3** | Design | `orka-research/src/service.rs` (1 235 righe) è già un god class alla prima release: gestisce 4 domini distinti (campaign, run, candidate, promotion) in un unico file | `orka-research` |

---

### 🟡 P2 — Media Priorità

| ID | Area | Descrizione | Crate coinvolto |
|---|---|---|---|
| **P2-1** | Layering | `orka-checkpoint` (L7 Orchestration) dipende da `orka-llm` (L2 AI): accoppiamento semanticamente errato tra persistenza e modelli LLM | `orka-checkpoint` |
| **P2-2** | Layering | `orka-eval` (L2 AI) dipende da `orka-skills` (L3 Execution): salto di layer in avanti | `orka-eval` |
| **P2-3** | Design | `ResearchService` usa `OnceLock<SkillRegistry>` per risolvere una dipendenza circolare d'inizializzazione: oggetto parzialmente inizializzato, fallimento silente se `bind_registry()` non chiamato | `orka-research` |
| **P2-4** | Dimensione | `orka-agent/src/node_runner.rs` (1 717 righe), `orka-worker/src/workspace_handler/mod.rs` (1 614 righe), `orka-cli/src/cmd/chat.rs` (1 600 righe): rimasti invariati dalla baseline, nessun refactoring effettuato | `orka-agent`, `orka-worker`, `orka-cli` |
| **P2-5** | Concorrenza | `A2aState.task_events: Arc<Mutex<HashMap>>` — singola mutex globale per tutti i task in-flight; in scenari ad alta concorrenza è un collo di bottiglia; considerare `DashMap` | `orka-a2a` |
| **P2-6** | Concorrenza | `AsyncServiceContainer` ha granularità lock globale (un RwLock per tutta la mappa); accettabile in bootstrap, problematico se usato come cache per-request | `orka-core` |

---

### 🟢 P3 — Bassa Priorità / Miglioramenti

| ID | Area | Descrizione | Crate coinvolto |
|---|---|---|---|
| **P3-1** | Qualità | 13 `.unwrap()` / `.expect()` in `orka-research/src/service.rs` (policy workspace: `unwrap_used = "warn"`) | `orka-research` |
| **P3-2** | Design | `orka-a2a/src/routes.rs` (779 righe) potrebbe essere suddiviso in moduli per handler (submit, stream, push) per migliorare la navigabilità | `orka-a2a` |
| **P3-3** | Design | I crate `orka-research` e `orka-a2a` hanno entrambi store duali (InMemory + Redis) — considerare un'astrazione condivisa per ridurre la duplicazione del pattern | `orka-research`, `orka-a2a` |
| **P3-4** | Concorrenza | Race condition documentata in `handle_push_notification_set` (gap tra `push_store.set()` e `task_events.lock()`) — gestita con fallback, ma non documentata con commento esplicito nel codice | `orka-a2a` |
| **P3-5** | Naming | `orka-research` è posizionato sotto "AI/Intelligence" nel workspace ma è funzionalmente un crate di Orchestration; il nome del layer nel commento del workspace è fuorviante | workspace `Cargo.toml` |

---

## Appendice — Matrice orka-core come leaf

| Verifica | Risultato |
|---|---|
| `orka-core` dipende da altri crate `orka-*`? | **No** ✅ |
| `orka-core/Cargo.toml` contiene riferimenti interni? | **No** ✅ |
| Qualche crate modifica tipi di `orka-core` senza passare dal trait? | Non rilevato ✅ |
| `orka-core` espone tipi da crate superiori (re-export)? | Non rilevato ✅ |

**Conclusione:** `orka-core` è e rimane un leaf puro. La sua stabilità come foundation del workspace è garantita.

---

*Report generato il 2026-03-25 da analisi statica del repository. Nessun file è stato modificato.*
