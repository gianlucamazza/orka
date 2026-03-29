# Orka — Test Coverage & Strategy Analysis

**Date:** 2026-03-25  
**Baseline reference:** `docs/internal/analysis-report.md` (2026-03-23)  
**Scope:** 40 crates in `./crates/`  
**Total test functions found:** 1,229

---

## 1. Inventario copertura per crate

La tabella seguente censisce ogni crate del workspace con il conteggio preciso di test inline (in `src/`), test di integrazione (in `tests/`), la presenza della directory `tests/`, e lo stato complessivo.

| Crate | Inline (`#[test]`) | Integration (`tests/`) | `tests/` dir | Status |
|---|---|---|---|---|
| orka-a2a | 24 | 28 | ✅ | 🟢 Buono |
| orka-adapter-custom | 4 | 4 | ✅ | 🟡 Minimale |
| orka-adapter-discord | 6 | 6 | ✅ | 🟡 Minimale |
| orka-adapter-slack | 7 | 8 | ✅ | 🟡 Minimale |
| orka-adapter-telegram | 44 | 6 | ✅ | 🟢 Buono |
| orka-adapter-whatsapp | 9 | 7 | ✅ | 🟡 Minimale |
| orka-agent | 75 | 21 | ✅ | 🟢 Buono |
| orka-auth | 12 | 3 | ✅ | 🟢 Buono |
| orka-bus | 10 | 4 | ✅ | 🟢 Buono |
| orka-checkpoint | 3 | 0 | ❌ | 🟡 Minimale |
| orka-circuit-breaker | 13 | 6 | ✅ | 🟢 Buono |
| orka-cli | 122 | 0 | ❌ | 🟡 Solo inline |
| orka-core | 134 | 9 | ✅ | 🟢 Eccellente |
| orka-eval | 4 | 22 | ✅ | 🟢 Buono |
| orka-experience | 26 | 9 | ✅ | 🟢 Buono |
| orka-gateway | 11 | 4 | ✅ | 🟢 Buono |
| orka-git | 12 | 0 | ❌ | 🟡 Solo inline |
| orka-guardrails | 24 | 11 | ✅ | 🟢 Buono |
| orka-http | 5 | 0 | ❌ | 🟡 Solo inline |
| orka-knowledge | 31 | 6 | ✅ | 🟢 Buono |
| orka-llm | 81 | 0 | ❌ | 🟡 Solo inline |
| orka-mcp | 3 | 12 | ✅ | 🟢 Buono |
| orka-memory | 3 | 5 | ✅ | 🟡 Minimale |
| orka-observe | 18 | 0 | ❌ | 🟡 Solo inline |
| orka-os | 98 | 0 | ❌ | 🟡 Solo inline |
| orka-prompts | 24 | 0 | ❌ | 🟡 Solo inline |
| orka-queue | 3 | 2 | ✅ | 🟡 Minimale |
| orka-research | 4 | 0 | ❌ | 🟡 Minimale |
| orka-sandbox | 5 | 14 | ✅ | 🟢 Buono |
| orka-scheduler | 26 | 0 | ❌ | 🟡 Solo inline |
| orka-secrets | 15 | 4 | ✅ | 🟢 Buono |
| orka-server | 3 | 38 | ✅ | 🟢 Eccellente |
| orka-session | 1 | 3 | ✅ | 🟡 Minimale |
| orka-skills | 22 | 6 | ✅ | 🟢 Buono |
| orka-wasm | 18 | 14 | ✅ | 🟢 Buono |
| orka-web | 19 | 0 | ❌ | 🟡 Solo inline |
| orka-worker | 27 | 13 | ✅ | 🟢 Buono |
| orka-workspace | 5 | 13 | ✅ | 🟢 Buono |

**Note metodologiche:**
- Il contatore inline include sia `#[test]` sia `#[tokio::test]` localizzati in `src/`.
- Il contatore integration include gli stessi attributi in `tests/`.
- Un crate "Solo inline" ha copertura reale ma senza separazione del contesto di test: nessun setup di fixtures esterne, nessun harness dedicato.
- Il totale non include i test nella directory `tests/` workspace-level (esterna a `crates/`).

---

## 2. Crate critici a zero test — stato aggiornato

Il report baseline (2026-03-23) segnalava **11 crate a 0 test**:
`orka-cli`, `orka-eval`, `orka-http`, `orka-llm`, `orka-observe`, `orka-os`, `orka-prompts`, `orka-scheduler`, `orka-web` (9 del gruppo originale, più `orka-checkpoint` e `orka-git` che erano già minimali).

### Situazione attuale

| Crate | Baseline (2026-03-23) | Stato attuale | Commit di riferimento |
|---|---|---|---|
| orka-cli | 0 ❌ | 122 inline ✅ | Precedente al baseline |
| orka-eval | 0 ❌ | 4+22=26 ✅ | Precedente al baseline |
| orka-http | 0 ❌ | 5 inline 🟡 | Post-baseline |
| orka-llm | 0 ❌ | 81 inline ✅ | Precedente al baseline |
| orka-observe | 0 ❌ | 18 inline ✅ | `7bb3de6` test(observe) |
| orka-os | 0 ❌ | 98 inline ✅ | Precedente al baseline |
| orka-prompts | 0 ❌ | 24 inline ✅ | Post-baseline |
| orka-scheduler | 0 ❌ | 26 inline ✅ | `8a11444` test(scheduler) |
| orka-web | 0 ❌ | 19 inline ✅ | Post-baseline |

**Conclusione:** **nessun crate è rimasto a zero test**. Il baseline era già obsoleto al momento della sua produzione rispetto a `orka-cli`, `orka-llm`, `orka-os`; i commit `8a11444` e `7bb3de6` hanno chiuso i gap di `orka-scheduler` e `orka-observe`.

**Avvertenza:** Per `orka-http` (5 test), `orka-checkpoint` (3 test), `orka-memory` (3 inline), `orka-session` (1 inline) la copertura rimane nominale: la quantità è sufficiente a soddisfare il predicato "non zero" ma insufficiente a coprire i percorsi critici.

---

## 3. Test recenti — orka-scheduler e orka-observe

### 3.1 orka-scheduler (commit `8a11444`)

**Distribuzione:** 26 test inline, 0 test di integrazione.

| File | Test | Tipologia |
|---|---|---|
| `src/memory_store.rs` | 7 `#[tokio::test]` | CRUD del `InMemoryScheduleStore` |
| `src/types.rs` | 2 `#[test]` | Serde + snapshot insta |
| `src/scheduler.rs` | 4 `#[test]` + 4 `#[tokio::test]` | Logica del loop di polling |
| `src/skills/schedule_create.rs` | 5 `#[tokio::test]` | Skill create (happy/sad path) |
| `src/skills/schedule_delete.rs` | 4 `#[tokio::test]` | Skill delete (happy/sad path) |
| `src/skills/schedule_list.rs` | 2 `#[tokio::test]` | Skill list |

**Qualità — punti di forza:**
- I test di `schedule_create` coprono i path principali: cron valido, one-shot, nome mancante, trigger mancante, cron malformato. Il pattern è ben strutturato con un helper `args()` che evita boilerplate.
- Il test `poll_and_execute_fires_due_task` verifica il contratto end-to-end del polling loop con un mock `ScheduleInvoker`, confermando che i task dovuti vengono eseguiti e che il `next_run` viene aggiornato per i ricorrenti (`recurring_schedule_updates_next_run`).
- Il test `one_shot_removed_after_execution` verifica la rimozione dei task one-shot post-esecuzione.

**Qualità — gap identificati:**
- **Redis store non testato**: `RedisScheduleStore` non ha test (dipendenza testcontainers assente). In produzione il codepath è diverso dall'`InMemoryScheduleStore`.
- **Concorrenza**: nessun test verifica il comportamento con schedulazioni multiple in parallelo o con cancellazione del `CancellationToken` durante l'esecuzione.
- **Timezone**: `schedule_create` accetta un campo `timezone` ma nessun test verifica che un timezone non valido generi errore.
- **Overflow del `next_run`**: nessun test verifica cosa accade con date molto lontane o nel passato.

### 3.2 orka-observe (commit `7bb3de6`)

**Distribuzione:** 18 test inline, 0 test di integrazione.

| File | Test | Tipologia |
|---|---|---|
| `src/audit_sink.rs` | 4 `#[tokio::test]` | JSONL audit log |
| `src/metrics.rs` | 2 `#[test]` | Prometheus recorder |
| `src/otel_sink.rs` | 7 `#[tokio::test]` | OpenTelemetry span mapping |
| `src/lib.rs` | 5 `#[tokio::test]`/`#[test]` | Fanout, factory, audit con file |

**Qualità — punti di forza:**
- I test OTel verificano non solo che gli span vengano emessi, ma la loro semantica: `SpanKind::Client` per `SkillCompleted`/`LlmCompleted`, `SpanKind::Server` per `AgentIteration`, `SpanKind::Internal` per `MessageReceived`, e lo stato `Error` per skill fallite con `error_message`.
- `all_event_variants_emit_without_panic` è un test di robustezza che itera su tutti i `DomainEventKind` — ottima protezione da regression su nuovi eventi aggiunti.
- Il test `fanout_broadcasts_to_all_sinks` verifica la propagazione a tutti i sink registrati, incluso la conservazione dell'ordine.
- `AuditSink` usa `NamedTempFile` per isolamento: nessuna dipendenza su filesystem di produzione.

**Qualità — gap identificati:**
- **`LogEventSink`**: non ha test di verifica del formato degli eventi loggati (solo smoke test).
- **`OtelSink` — span attributes**: verificati solo per `LlmCompleted` (`gen_ai.request.model`). Attributi custom di altri eventi (es. `agent.iteration`, `error.category`) non sono validati.
- **`create_event_sink`**: i branch `otel` (con esportatore reale) e `redis` non sono testati; solo `log` e `audit` lo sono.
- **Concurrency**: il fanout non testa l'ordine garantito o la gestione di sink lenti.

---

## 4. Nuovi crate — orka-research e orka-a2a

### 4.1 orka-research (untracked)

**Struttura:** `src/{lib.rs, service.rs, skills.rs, store.rs, types.rs, util.rs}` — nessuna `tests/` directory.

**Test presenti:** 4 test inline in `service.rs`:
- `extract_metric_parses_first_capture_group` — unit test su regex di estrazione metriche.
- `compare_against_metric_discards_regressions` — verifica logica comparativa.
- `run_campaign_creates_kept_candidate` — test asincrono con mock stores che verifica il flow completo di una campaign run.
- `promote_candidate_requires_explicit_approval` — verifica il gate di approvazione.

**Qualità:** I due test async sono notevoli: utilizzano mock in-memory di `CheckpointStore`, `ScheduleStore`, `ResearchStore`, e uno `SkillRegistry` con una skill stub. Il flow `run_campaign_creates_kept_candidate` tocca `create_campaign`, `trigger_run`, e la promozione automatica con metrica soddisfatta.

**Gap critici:**
- `ResearchService` espone ~15 metodi pubblici; solo 4 sono testati.
- Non ci sono test per: campaign validation con `editable_paths` non validi, `verify_candidate` reale (subprocess), branch protection, schedule integration, `list_runs`, `get_candidate`.
- `orka-server/tests/api_research.rs` copre i layer HTTP ma non la logica di business dell'`orka-research` crate stesso.

### 4.2 orka-a2a

**Struttura:** `src/` + `tests/a2a_test.rs` — crate ben testato.

**Test presenti:** 24 inline + 28 integration = **52 totale**.

| Componente | File | Test |
|---|---|---|
| Tipi A2A (serializzazione) | `src/types.rs` | 8 `#[test]` |
| Agent card | `src/agent_card.rs` | 7 `#[test]` |
| Discovery service | `src/discovery.rs` | 4 `#[tokio::test]` |
| Push notification store | `src/push_store.rs` | 5 `#[tokio::test]` |
| Integration (HTTP handler) | `tests/a2a_test.rs` | 28 `#[tokio::test]` |

**Qualità:** I test di integrazione coprono tutti i metodi JSON-RPC A2A v1.0: `tasks/send`, `tasks/get`, `tasks/cancel`, `tasks/list`, `tasks/resubscribe`, `tasks/sendSubscribe` (streaming SSE), `tasks/pushNotification/set/get`. Il file `a2a_test.rs` è il più completo del workspace con 700+ righe e test di casi sia happy che error (task non trovato, metodo sconosciuto, stato invalido per cancel, schema SSE).

**Gap:**
- Non ci sono test per la federazione multi-agente (route A2A → A2A).
- I test di push notification verificano il store in-memory ma non l'effettivo invio HTTP.

---

## 5. Proptest e property-based testing

### Situazione attuale

**⚠️ DISCREPANZA CRITICA CON IL BASELINE**

Il report del 2026-03-23 indicava **~586 asserzioni proptest**. L'analisi attuale rileva:

- **1 unico file** usa `proptest`: `crates/orka-knowledge/src/chunking.rs`
- **1 blocco `proptest!`** con **3 test** e ~5 `prop_assert!`/`prop_assert_eq!`
- **0 altri file** nel workspace usano `proptest::prelude::*` o il macro `proptest!`

Il totale di linee con termini proptest-correlati (incluse `Strategy`, `Arbitrary`, `ProptestConfig`) è **70**, di cui la maggioranza sono import e definizioni nel singolo file `chunking.rs`.

**Conclusione:** Il dato "~586 asserzioni proptest" nel baseline era **errato**. L'ordine di grandezza era sbagliato di due decimi. Il testing property-based è praticamente assente nel progetto.

### Valutazione dei 3 proptest esistenti

```rust
// orka-knowledge/src/chunking.rs
proptest! {
    fn text_with_words_produces_at_least_one_chunk(...)  // ✅ Utile
    fn empty_text_always_empty(...)                       // ✅ Utile
    fn overlap_never_panics_on_unicode(...)               // ✅ Ottimo (robustezza Unicode)
}
```

I tre test sono ben scelti: coprono le invarianti chiave di `split_text`. Il terzo (`overlap_never_panics_on_unicode`) è particolarmente valido perché usa `\\PC{10,500}` (caratteri stampabili Unicode arbitrari) come input.

### Gap proptest

Aree dove il property-based testing apporterebbe valore massimo:

| Area | Invariante da verificare |
|---|---|
| `orka-core/src/types.rs` | Serde round-trip su tutti i tipi pubblici |
| `orka-llm/src/context.rs` | `truncate_history`: len output ≤ budget; primo turno sempre preservato |
| `orka-auth/src/jwt.rs` | JWT con claim arbitrari → validazione deterministica |
| `orka-scheduler/src/types.rs` | `next_cron_timestamp`: monotonia, no panic su cron arbitrari |
| `orka-guardrails/src/regex_filter.rs` | No panic su regex arbitrarie |

---

## 6. Snapshot testing (insta)

### Situazione attuale

**Snapshot totali nel workspace:** 1

```
crates/orka-scheduler/src/snapshots/
  orka_scheduler__types__tests__schedule.snap
```

**Unico punto di utilizzo:**
```rust
// crates/orka-scheduler/src/types.rs:57
insta::assert_json_snapshot!("schedule", sample_schedule());
```

Lo snapshot serializza un `Schedule` campione con tutti i campi valorizzati (cron, timezone, args, next_run, created_at). Il test `#[test] fn schedule_snapshot()` non è di tipo `#[tokio::test]` poiché `sample_schedule()` è sincrono.

**Valutazione:**
- Lo snapshot è utile ma copre solo la struttura di un dato statico. Non verifica serializzazione di edge case (schedule con `run_at` ma senza `cron`, `args` null, `message` valorizzato).
- L'utilizzo di `insta` è quasi simbolico: 1 snapshot su un framework di 40 crate è sottoutilizzato.
- Aree ad alto valore per snapshot test: risposte JSON dell'API REST (es. `/api/v1/skills`, `/api/v1/graph`), output del `NodeRunner` con tool calls, formato JSONL di `AuditSink`.

---

## 7. Test di integrazione

### 7.1 orka-server/tests/

6 file di test, 38 test di integrazione totali. Tutti usano `tower::ServiceExt::oneshot` — nessun listener TCP, esecuzione completamente in-memory.

| File | Test | Aree coperte |
|---|---|---|
| `api_auth.rs` | ~10 | Auth middleware: API key valida/invalida/assente, A2A auth, route pubbliche |
| `api_health.rs` | ~5 | `/health/live`, `/health/ready`, `/metrics` |
| `api_management.rs` | ~15 | Skills, sessions, DLQ, workspaces, graph, experience, schedules |
| `api_research.rs` | ~8 | Research campaigns CRUD, pause, run, promote/approve/reject |
| `api_roundtrip.rs` | ~2 | WebSocket roundtrip, session reuse |
| `e2e_flow.rs` | 1 | **Full flow**: adapter → bus → gateway → queue → worker → bus |

#### e2e_flow.rs — analisi approfondita

`full_message_flow_echo` è il test di sistema più significativo: verifica l'intera pipeline di messaggi in-memory con `InMemoryBus`, `InMemoryQueue`, `InMemorySessionStore`, `Gateway`, `WorkerPool` e `EchoHandler`. Verifica:
1. Dispatch dell'inbound message attraverso il gateway.
2. Accodamento in `InMemoryQueue`.
3. Elaborazione del worker con echo reply.
4. Ricezione della risposta outbound.
5. Emissione di `DomainEvent` (`MessageReceived`, `AgentIterationCompleted`).
6. Creazione/riutilizzo della sessione.

**Gap del layer server:**
- Le route `/api/v1/runs/{run_id}/checkpoints`, `/api/v1/runs/{run_id}/approve`, `/api/v1/runs/{run_id}/reject` (5 endpoint checkpoints) non hanno test.
- Il WebSocket endpoint (chat streaming) non ha test dedicati in `api_roundtrip.rs`.
- Le route A2A nel server (`/a2a`, `/.well-known/agent.json`) sono testate in `api_auth.rs` per la sola autenticazione, ma non per il comportamento funzionale.

### 7.2 Test con testcontainers

7 crate usano `testcontainers` (Redis e/o Qdrant):

| Crate | Container | Test |
|---|---|---|
| `orka-bus` | Redis | Pub/sub con broker reale |
| `orka-checkpoint` | Redis | Save/load/list/delete checkpoint |
| `orka-knowledge` | Qdrant | Vector store upsert/query |
| `orka-memory` | Redis | Store/retrieve memory entries |
| `orka-queue` | Redis | Enqueue/dequeue |
| `orka-secrets` | Redis | Encrypt/decrypt, rotation |
| `orka-session` | Redis | Session create/update/delete |

**Qualità:** I test testcontainers sono corretti ma minimi (2-5 test per crate). Coprono le operazioni CRUD fondamentali ma non scenari avanzati (TTL scaduto, connessione Redis persa, failover, large payloads).

---

## 8. Gap analysis — percorsi critici senza test

### 8.1 Autenticazione

| Percorso | Stato |
|---|---|
| JWT HMAC validation | ✅ `orka-auth/src/jwt.rs` (4 test) |
| JWT RSA validation | ⚠️ Solo smoke test, nessun test con chiave reale |
| API key hashing (SHA-256) | ✅ 3 test inline |
| Middleware Axum extraction | ✅ `tests/middleware_test.rs` + server `api_auth.rs` |
| JWT expired token | ❌ **Assente** |
| JWT con claims custom | ❌ **Assente** |
| Bearer token in Authorization header | ⚠️ Testato solo via middleware mock |

### 8.2 Routing (server)

| Route | Stato |
|---|---|
| `/health/*` | ✅ `api_health.rs` |
| `/api/v1/skills` | ✅ `api_management.rs` |
| `/api/v1/sessions` | ✅ `api_management.rs` |
| `/api/v1/dlq` | ✅ `api_management.rs` |
| `/api/v1/schedules` | ✅ `api_management.rs` (solo "not enabled") |
| `/api/v1/runs/{id}/checkpoints` | ❌ **Nessun test** |
| `/api/v1/runs/{id}/approve` | ❌ **Nessun test** |
| `/api/v1/runs/{id}/reject` | ❌ **Nessun test** |
| `/api/v1/runs/{id}/status` | ❌ **Nessun test** |
| `/.well-known/agent.json` | ⚠️ Solo test auth, non funzionale |
| `/a2a` (JSON-RPC) | ⚠️ Solo test auth |

### 8.3 Agent execution

| Percorso | Stato |
|---|---|
| `node_runner.rs` (LLM tool loop, 1,717 linee) | 🟡 10 test inline (iterazione base + tool call) |
| `executor.rs` (855 linee, graph execution) | ❌ **Zero test** |
| `handoff.rs` (31 linee) | ❌ **Zero test** |
| `planner.rs` (plan/reduce cycle) | ✅ 4 test |
| `reducer.rs` (history management) | ✅ 10 test |
| `graph.rs` (node routing) | ✅ 2 test |
| `context_adapters.rs` | ❌ **Zero test** |
| `graph_executor_test.rs` (integration) | ✅ 21 test async |

**⚠️ Gap critico:** `executor.rs` (855 linee) è il cuore dell'esecuzione del grafo agente — gestisce checkpoint save/restore, interrupt handling, approval flow, loop di esecuzione. Non ha **nessun test**.

### 8.4 Checkpoint & interruzione

| Percorso | Stato |
|---|---|
| `RedisCheckpointStore` CRUD | ✅ 3 test testcontainers |
| Checkpoint save durante esecuzione | ❌ **Nessun test** |
| Restore da checkpoint e resume | ❌ **Nessun test** |
| `InterruptReason::AwaitingApproval` → approve → resume | ❌ **Nessun test** |
| `InterruptReason::AwaitingApproval` → reject → terminal | ❌ **Nessun test** |
| Server route `/approve` / `/reject` | ❌ **Nessun test** |

**⚠️ Gap critico:** Il sistema di checkpoint è una feature centrale (commit `93baed4`) ma il percorso completo "esecuzione → interrupt → approval → resume" non è coperto da nessun test.

### 8.5 LLM e streaming

| Percorso | Stato |
|---|---|
| Context truncation (`orka-llm/src/context.rs`) | ✅ 19 test inline |
| Circuit breaker (`orka-llm/src/router.rs`) | ✅ 2 test inline |
| `LlmRouter` con model selection | ⚠️ Solo mock di `LlmClient` |
| Anthropic API parsing | 🟡 Parziale (stream_consumer.rs ha test) |
| OpenAI API parsing | 🟡 Parziale |
| Adaptive thinking / reasoning blocks | ❌ **Nessun test end-to-end** |
| Streaming con tool calls interleaved | ❌ **Nessun test** |

---

## 9. Findings riassuntivi

### P0 — Critico (rischio blocco operativo)

| ID | Finding | Crate/File |
|---|---|---|
| P0-1 | `executor.rs` (855 linee) zero test: gestisce checkpoint save/restore, interrupt, approval flow, loop principale di esecuzione del grafo agente | `orka-agent/src/executor.rs` |
| P0-2 | Percorso completo checkpoint→interrupt→approve/reject→resume non testato. Unica copertura è il CRUD Redis in isolamento | `orka-checkpoint`, `orka-agent`, `orka-server` |

### P1 — Elevato (rischio regressione silente in produzione)

| ID | Finding | Crate/File |
|---|---|---|
| P1-1 | Route `/api/v1/runs/{id}/approve` e `/reject` non testate. La logica di approvazione in produzione non ha safety net | `orka-server/src/router/checkpoints.rs` |
| P1-2 | `orka-research` ha solo 4 test inline su ~15 metodi pubblici di `ResearchService`. Il flusso di esecuzione reale (subprocess verification, branch management) è zero-tested | `orka-research/src/service.rs` |
| P1-3 | JWT expired token e claim validation non testati: un token scaduto o malformato potrebbe passare in condizioni di edge case | `orka-auth/src/jwt.rs` |
| P1-4 | `orka-scheduler/RedisScheduleStore` non testato: in produzione usa Redis ma solo `InMemoryScheduleStore` ha test | `orka-scheduler/src/redis_store.rs` |
| P1-5 | Property-based testing assente per invarianti critiche (`context.rs` token budgeting, serde round-trip dei tipi core). Il baseline indicava 586 asserzioni proptest — **il dato era errato**: esistono solo 3 proptest in 1 file | `orka-knowledge/src/chunking.rs` |

### P2 — Medio (debito tecnico accumulabile)

| ID | Finding | Crate/File |
|---|---|---|
| P2-1 | 9 crate con solo test inline e senza `tests/` dir: `orka-cli`, `orka-git`, `orka-http`, `orka-llm`, `orka-observe`, `orka-os`, `orka-prompts`, `orka-scheduler`, `orka-web`. L'assenza di test di integrazione impedisce test con fixtures e setup esterni | Multipli |
| P2-2 | `insta` usato in un solo punto (1 snapshot). Il framework è un overhead di dipendenza non sfruttato; oppure va esteso alle risposte JSON delle API | `orka-scheduler/src/types.rs` |
| P2-3 | `orka-checkpoint` ha solo 3 test (tutti testcontainers-only), nessun test senza Redis disponibile. Manca un `InMemoryCheckpointStore` per test fast | `orka-checkpoint` |
| P2-4 | Test testcontainers per bus/queue/session/memory/secrets coprono solo CRUD base. TTL scaduto, connessione persa, reconnect non sono testati | Multipli |
| P2-5 | `orka-http` ha 5 test inline ma copre solo i guard/validation. Le skill HTTP reali (execute request, redirect, timeout) non sono testate | `orka-http/src/skills/request.rs` |
| P2-6 | `orka-a2a`: invio HTTP reale nelle push notification non testato (solo store in-memory) | `orka-a2a/src/push_store.rs` |
| P2-7 | `orka-agent/src/handoff.rs` e `context_adapters.rs` zero test | `orka-agent/src/` |

### P3 — Basso (miglioramento qualitativo)

| ID | Finding | Crate/File |
|---|---|---|
| P3-1 | `orka-scheduler`: nessun test per timezone invalido, `next_run` overflow, date nel passato | `orka-scheduler` |
| P3-2 | `orka-observe/OtelSink`: solo `gen_ai.request.model` attribute verificato; altri span attributes non validati | `orka-observe/src/otel_sink.rs` |
| P3-3 | Benchmark limitato a 1 file (`benches/message_bus.rs`). Assenti benchmark per: LLM token processing, context truncation, serializzazione tipi core | `benches/` |
| P3-4 | `orka-worker` ha 4 file di test ma nessuno verifica il comportamento del dispatcher con `max_retries > 0` in condizioni di failure dell'`AgentHandler` (solo `EchoHandler` usato nel e2e) | `orka-worker` |
| P3-5 | Il worktree residuo in `.orka-worktrees/research-e2e-doc-test-20260325210713/` contiene una copia dello snapshot insta — potenziale source of truth duplicata | `.orka-worktrees/` |

---

## Appendice: metriche aggregate

| Metrica | Valore |
|---|---|
| Crate totali analizzati | 38 (+2 nuovi: orka-research, orka-a2a) |
| Crate a zero test (baseline 2026-03-23) | 11 |
| Crate a zero test (stato attuale) | **0** |
| Test totali (inline + integration) | **1,229** |
| Test inline (`src/`) | ~891 |
| Test di integrazione (`tests/`) | ~338 |
| Crate con `tests/` dir | 28/40 (70%) |
| Test property-based (proptest) | **3** (in 1 file) |
| Snapshot insta | **1** |
| File con testcontainers | 7 |
| Percorsi critici senza test (P0+P1) | 7 |
