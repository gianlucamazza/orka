# Orka Codebase Analysis Report

**Date:** 2026-03-23
**Version analyzed:** 1.0.0
**Commit:** HEAD
**MSRV:** 1.91
**Edition:** 2024

---

## Executive Summary

The Orka codebase is an AI agent orchestration framework written in Rust with
**43 workspace packages** (`40` under `crates/` plus `3` under `sdk/`). The
architecture is modular and well-layered, though it has areas with incomplete
test coverage and some technical debt in specific crates.

### Key Metrics

| Metric | Value | Status |
|--------|-------|--------|
| Total lines of code (src) | ~58,000 | ✅ |
| Workspace packages | 43 | ✅ |
| Unwrap/expect/todo! occurrences | ~1,287 | 🟡 P2 |
| Files >500 lines | 12 | 🟡 P2 |
| Avg test files per crate | 1.2 | 🟡 P2 |
| Crates without tests | 11 | 🔴 P1 |
| Unsafe blocks | 5 | ✅ |
| Doc warnings | 7 | 🟢 |

---

## Area 1 — Architecture and Abstraction Design

### 1.1 Object-Safety and Trait Bounds

**Status:** ✅ **Excellent**

All core traits are correctly defined with `Send + Sync + 'static` bounds:

```rust
// crates/orka-core/src/traits.rs
#[async_trait]
pub trait ChannelAdapter: Send + Sync + 'static { ... }
pub trait MessageBus: Send + Sync + 'static { ... }
pub trait SessionStore: Send + Sync + 'static { ... }
pub trait MemoryStore: Send + Sync + 'static { ... }
```

Use of `async-trait` is consistent and necessary for dynamic dispatch. The `Skill` trait uses `serde_json::Value` as an associated type for input/output, preserving flexibility.

### 1.2 Service Container (DI)

**Status:** ✅ **Removed** *(as of 2026-03-28)*

`orka-core/src/container.rs` (ServiceContainer, LazyContainer, AsyncServiceContainer) has been removed. Bootstrap wiring now happens directly in `orka-server/src/bootstrap.rs` without a DI container. See `docs/internal/architecture-analysis-2026-03-25.md` §4 for the historical analysis.

### 1.3 Data/Logic Separation

**Status:** ✅ **Good**

- `orka-core/src/types.rs` (~1,340 lines): contains only types and data structures
- `orka-agent/src/agent.rs`: separates `Agent` (data) from `AgentRunner` (logic)
- `orka-llm/src/context.rs`: token budget and context management well isolated

### 1.4 Circular Dependencies

**Status:** ✅ **None detected**

Dependency graph:
```
orka-core ← all others (leaf)
orka-bus, orka-queue, orka-session → orka-core
orka-agent → orka-llm, orka-core, orka-skills
orka-server → all (orchestrator)
```

### 1.5 Large Files

| File | Lines | Assessment |
|------|-------|------------|
| `orka-worker/src/workspace_handler/` | ~2,172 | 🟡 Excessive |
| `orka-cli/src/cmd/chat.rs` | ~1,361 | 🟡 Excessive |
| `orka-core/src/types.rs` | ~1,340 | 🟡 Acceptable (data-only) |
| `orka-os/src/skills/fs.rs` | ~1,097 | 🟢 Aggregated skills |

Note: `orka-core/src/config.rs` was previously a 2,712-line god module — it has since been split into `crates/orka-core/src/config/` with 18 domain-specific modules, and as of 2026-03-28 further reduced to 3 modules (`agent`, `defaults`, `primitives`): all domain config now lives in owning crates (e.g. `orka-os/src/config.rs`, `orka-auth/src/config.rs`), top-level runtime types in `orka-config/src/runtime.rs`.

---

## Area 2 — Code Quality and Technical Debt

### 2.1 Panic Points (unwrap/expect)

**Total:** ~1,287 occurrences

**Distribution (estimated):**
- Tests: ~60% (acceptable)
- Production: ~40% (concerning)

`expect_used` and `unwrap_used` are configured as `warn` in clippy, but violations are present.

### 2.2 TODO/FIXME/HACK

**Status:** ✅ **Excellent**

No TODO/FIXME/HACK/XXX found in source code.

### 2.3 Unjustified `#[allow]` Attributes

**Status:** 🟡 **Needs review**

```
crates/orka-adapter-slack/src/lib.rs          #[allow(dead_code)]
crates/orka-adapter-telegram/src/types.rs     #[allow(dead_code)] ×5
crates/orka-adapter-telegram/src/webhook.rs   #[allow(clippy::too_many_arguments)]
crates/orka-core/src/testing.rs               #[allow(clippy::type_complexity)]
crates/orka-gateway/src/lib.rs                #[allow(clippy::too_many_arguments)]
crates/orka-guardrails/src/chain.rs           #[allow(clippy::should_implement_trait)]
crates/orka-worker/src/lib.rs                 #[allow(clippy::too_many_arguments)]
```

`dead_code` on adapters indicates partially implemented features. Each should be documented with an explanatory comment.

### 2.4 Unsafe Code

**Status:** ✅ **Minimal and justified**

5 total occurrences:
- `orka-cli/src/completion.rs:526,528`: test-only env var manipulation
- `orka-os/src/skills/env.rs:202,212`: test-only env var manipulation
- `orka-os/src/lib.rs:39`: `libc::prctl` for sandboxing (justified)

All unsafe code is isolated to tests or necessary Linux system calls.

---

## Area 3 — Test Strategy and Coverage

### 3.1 Test Inventory by Crate

| Crate | Test Files | Test Functions | Status |
|-------|-----------|----------------|--------|
| orka-a2a | 1 | ~5 | 🟡 Minimal |
| orka-adapter-custom | 1 | ~3 | 🟡 Minimal |
| orka-adapter-discord | 1 | ~3 | 🟡 Minimal |
| orka-adapter-slack | 1 | ~3 | 🟡 Minimal |
| orka-adapter-telegram | 1 | ~5 | 🟢 OK |
| orka-adapter-whatsapp | 1 | ~3 | 🟡 Minimal |
| orka-agent | 1 | ~10 | 🟢 OK |
| orka-auth | 1 | ~8 | 🟢 OK |
| orka-bus | 1 | ~6 | 🟢 OK |
| orka-circuit-breaker | 1 | ~5 | 🟡 Minimal |
| orka-cli | **0** | 0 | 🔴 Critical |
| orka-core | 1 | ~15 | 🟢 OK |
| orka-eval | **0** | 0 | 🔴 Critical |
| orka-experience | 1 | ~5 | 🟡 Minimal |
| orka-gateway | 2 | ~8 | 🟢 OK |
| orka-guardrails | 1 | ~5 | 🟡 Minimal |
| orka-http | **0** | 0 | 🔴 Critical |
| orka-knowledge | 1 | ~5 | 🟡 Minimal |
| orka-llm | **0** | 0 | 🔴 Critical |
| orka-mcp | 2 | ~8 | 🟢 OK |
| orka-memory | 1 | ~5 | 🟡 Minimal |
| orka-observe | **0** | 0 | 🔴 Critical |
| orka-os | **0** | 0 | 🔴 Critical |
| orka-prompts | **0** | 0 | 🔴 Critical |
| orka-queue | 1 | ~5 | 🟡 Minimal |
| orka-sandbox | 1 | ~5 | 🟡 Minimal |
| orka-scheduler | **0** | 0 | 🔴 Critical |
| orka-secrets | 1 | ~5 | 🟡 Minimal |
| orka-server | 6 | ~20 | 🟢 OK |
| orka-session | 1 | ~5 | 🟡 Minimal |
| orka-skills | 1 | ~5 | 🟡 Minimal |
| orka-wasm | 2 | ~8 | 🟢 OK |
| orka-web | **0** | 0 | 🔴 Critical |
| orka-worker | 4 | ~15 | 🟢 OK |
| orka-workspace | 1 | ~5 | 🟡 Minimal |

### 3.2 Testing Infrastructure

**Status:** ✅ **Excellent**

- `testcontainers` for integration tests with Redis/Qdrant
- `proptest` for property-based testing (~586 assertions)
- `insta` for snapshot testing
- `orka-core/src/testing.rs`: 636 lines of test doubles and mocks

---

## Area 4 — Security and Robustness

### 4.1 Authentication (orka-auth)

**Status:** ✅ **Correctly implemented**

| Feature | Status | Notes |
|---------|--------|-------|
| JWT (HMAC) | ✅ | `jwt.rs:79` — validation via `jsonwebtoken` |
| JWT (RSA) | ✅ | `jwt.rs:21` — public key validation |
| API Key | ✅ | `api_key.rs:13` — SHA-256 hashing |
| Axum Middleware | ✅ | `middleware.rs:228` — header extraction |

### 4.2 Secrets (orka-secrets)

**Status:** ✅ **Excellent**

- **AES-256-GCM** encryption at rest
- **zeroize** for secure memory clearing
- Key rotation supported (`rotation.rs:356`)

### 4.3 Sandboxing (orka-sandbox, orka-os)

**Status:** 🟡 **Partial**

| Feature | Status |
|---------|--------|
| Process isolation | ✅ `process.rs:169` |
| Timeout enforcement | ✅ `config.shell_timeout_secs` |
| Command allowlist | ✅ `allowed_shell_commands` |
| Seccomp/landlock | ⚠️ `prctl` check present but limited |
| SSRF protection | ⚠️ Config-dependent |

`shell_words::split` used correctly to prevent shell injection (line 117).

### 4.4 Guardrails (orka-guardrails)

**Status:** 🟡 **Structure present, basic implementation**

Implementations:
- `keyword.rs:86` — basic keyword filtering
- `regex_filter.rs:150` — regex matching
- `code_filter.rs:187` — code detection (heuristic)
- `chain.rs:124` — chaining multiple guardrails

No LLM-based guardrails (toxicity detection, prompt injection detection).

### 4.5 Dependency Scanning

**Status:** ✅ **Fixed** — `deny.toml` migrated to v2 format.

---

## Area 5 — Performance and Scalability

### 5.1 Hot Path Analysis

| File | Lines | Hot Path | Notes |
|------|-------|----------|-------|
| `orka-agent/src/node_runner.rs` | 953 | LLM tool loop | Streaming + token mgmt |
| `orka-worker/src/workspace_handler/` | ~2,172 | Message dispatch | To optimize |
| `orka-core/src/types.rs` | ~1,340 | Serialization | OK — data only |

### 5.2 Memory Efficiency

**Status:** ✅ **Good**

`Arc<str>` used instead of `String` for immutable identifiers (e.g. `AgentId`).

### 5.3 Connection Pooling

**Status:** ✅ **Implemented**

`deadpool-redis` used across bus, queue, memory, scheduler crates. Pool size uses library defaults in most crates.

### 5.4 Token Budget Management

**Status:** ✅ **Implemented**

`orka-llm/src/context.rs` provides `available_history_budget_with_hint` and `truncate_history_with_hint` with support for `TokenizerHint::Claude`, `TokenizerHint::OpenAi`, `TokenizerHint::Tiktoken`.

### 5.5 Benchmarks

**Status:** 🟡 **Minimal**

Only `benches/message_bus.rs` (149 lines). Missing: LLM token processing, context truncation, serialization/deserialization.

---

## Area 6 — Developer Experience and Maintainability

### 6.1 Documentation

**Status:** 🟡 **Good with warnings**

7 documentation warnings — all minor:
- Private item links in public docs
- Unresolved links
- Unclosed HTML tags

### 6.2 CLI Ergonomics

**Status:** ✅ **Excellent**

- Clap with derive macros
- Shell completion support
- Markdown rendering in terminal
- TUI dashboard with real-time metrics

### 6.3 Examples and Demos

**Status:** ✅ **Good**

```
examples/
  basic_bot/      # Simple echo bot
  custom_skill/   # Custom skill implementation
  multi_agent/    # Multi-agent workflow
  wasm_plugin/    # WASM plugin

demo/
  *.gif           # Demo recordings
  *.tape          # VHS recording scripts
```

### 6.4 Dockerfile

**Status:** ✅ **Production-ready**

Multi-stage build with `cargo-chef`, optimized layer caching, `mold` linker, security hardening (`read_only`, `no-new-privileges`).

### 6.5 CI Pipeline

**Status:** ✅ **Comprehensive**

- `ci.yml`: commitlint, fmt (nightly), clippy, cargo audit, cargo deny, build, unit tests, integration tests (Redis + Qdrant), MSRV check (1.91), coverage
- `packaging.yml`: Debian and Fedora package linting
- `release.yml`: release automation
- `typos.yml`: typo checking

---

## Area 7 — Feature Completeness

### 7.1 Feature Matrix

| Crate | Status | Lines | Notes |
|-------|--------|-------|-------|
| **CORE** | | | |
| orka-core | 🟢 Done | ~4,500 | Types, traits, config primitives (agent/defaults); container removed 2026-03-28 |
| orka-circuit-breaker | 🟢 Done | ~200 | Pattern implemented |
| **INFRASTRUCTURE** | | | |
| orka-bus | 🟢 Done | ~400 | Redis Streams |
| orka-queue | 🟢 Done | ~300 | Priority queue |
| orka-session | 🟢 Done | ~300 | Session store |
| orka-memory | 🟢 Done | ~350 | Semantic memory |
| orka-secrets | 🟢 Done | ~450 | AES-256-GCM + rotation |
| orka-scheduler | 🟡 Partial | ~250 | Cron base, no distributed coordinator |
| orka-observe | 🟡 Partial | ~400 | Metrics OK, tracing partial |
| **AI / INTELLIGENCE** | | | |
| orka-llm | 🟢 Done | ~2,800 | Anthropic, OpenAI, Ollama |
| orka-knowledge | 🟢 Done | ~600 | RAG with Qdrant |
| orka-prompts | 🟢 Done | ~900 | Handlebars templating |
| orka-guardrails | 🟡 Partial | ~600 | Keyword/regex, no LLM-based |
| orka-experience | 🟡 Partial | ~1,800 | Structure complete, minimal tests |
| orka-eval | 🟡 Partial | ~400 | Framework implemented, no tests |
| **EXECUTION** | | | |
| orka-skills | 🟢 Done | ~800 | Registry + macro |
| orka-wasm | 🟢 Done | ~700 | Wasmtime Component Model |
| orka-sandbox | 🟢 Done | ~600 | Process isolation |
| orka-mcp | 🟢 Done | ~700 | Model Context Protocol |
| orka-a2a | 🟡 Partial | ~550 | Routes present, coverage thin |
| **ADAPTERS** | | | |
| orka-adapter-telegram | 🟢 Done | ~900 | Full (polling, webhook, media) |
| orka-adapter-discord | 🟢 Done | ~554 | Complete |
| orka-adapter-slack | 🟢 Done | ~571 | Complete |
| orka-adapter-whatsapp | 🟢 Done | ~560 | Complete |
| orka-adapter-custom | 🟢 Done | ~300 | HTTP + WebSocket |
| **NETWORKING** | | | |
| orka-http | 🟢 Done | ~400 | HTTP client |
| orka-web | 🟢 Done | ~500 | Tavily/Brave/SearXNG search |
| orka-auth | 🟢 Done | ~550 | JWT + API Key |
| orka-os | 🟢 Done | ~2,200 | Linux integration |
| **ORCHESTRATION** | | | |
| orka-agent | 🟢 Done | ~2,500 | Multi-agent graph |
| orka-workspace | 🟢 Done | ~1,200 | Workspace loading |
| orka-gateway | 🟢 Done | ~600 | Rate limiting + dedup |
| orka-worker | 🟢 Done | ~3,200 | Worker pool |
| orka-cli | 🟢 Done | ~3,500 | Full CLI + TUI |
| orka-server | 🟢 Done | ~2,800 | HTTP server + bootstrap |

### 7.2 Priority Backlog

| ID | Priority | Issue | Effort | Impact |
|----|----------|-------|--------|--------|
| 1 | **P1** | Add tests to crates with zero coverage | 8h | 🟡 Quality |
| 2 | **P1** | Complete `orka-eval` test coverage | 4h | 🟡 Quality |
| 3 | **P2** | Reduce unwrap/expect in production code | 8h | 🟡 Robustness |
| 4 | **P2** | Add critical benchmarks (LLM, serialization) | 4h | 🟡 Performance |
| 5 | **P2** | Document `#[allow]` attributes | 1h | 🟡 Maintainability |
| 6 | **P3** | Implement LLM-based guardrails | 8h | 🟢 Feature |
| 7 | **P3** | Optimize `workspace_handler` | 4h | 🟡 Performance |

---

## Conclusions

### Strengths

1. **Modular architecture** — well-layered, no circular dependencies
2. **Security** — AES-256-GCM, zeroize, JWT, SSRF protection
3. **Tooling** — comprehensive CI, Docker, Justfile
4. **Extensibility** — WASM plugins, MCP, A2A, soft skills
5. **Minimal unsafe** — 5 occurrences, all justified

### Immediate Gaps

1. **11 crates without any tests** — regression risk
2. **High unwrap/expect count in production code** — robustness risk
3. **LLM-based guardrails missing** — feature gap

---

*Report produced by automated codebase analysis — 2026-03-23. Updated 2026-03-25.*
