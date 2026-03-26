# Orka Security & Robustness Analysis Report

**Date:** 2026-03-25  
**Scope:** Full workspace (40 crates)  
**Focus:** A2A v1.0, research endpoints (added post-baseline), plus all existing security surfaces

---

## 1. cargo audit — Dependency Vulnerability Scan

`cargo-audit` and `cargo-deny check advisories` are not installed in this environment. The analysis relies on direct version inspection of `Cargo.lock` against known advisories.

### cargo-deny Configuration (`deny.toml`)
The workspace has `cargo-deny` configured with:
- `yanked = "deny"` — yanked crates are rejected at CI time ✅
- `unknown-registry = "deny"` and `unknown-git = "deny"` — supply-chain protection ✅
- License allowlist covers standard permissive licenses ✅

### Key Dependency Versions Inspected

| Crate | Version | Notes |
|---|---|---|
| `tokio` | 1.50.0 | Recent; no known unpatched CVEs |
| `h2` | 0.4.13 | Recent; covers RST flood fix (GHSA-qwhd) present in 0.4+ |
| `rustls` | 0.23.37 | Recent; no known unpatched CVEs |
| `openssl-sys` | 0.10.76 | Bound to system OpenSSL; runtime version determines exposure |
| `ring` | 0.8.53 | Recent release |
| `jsonwebtoken` | 0.28.3 | Recent; alg:none attack not possible (library enforces algorithm on decode) |

**Finding:** No known critical CVEs identified in the versions present. However, `openssl-sys` delegates to the system-installed OpenSSL library — the actual security posture depends on the deployment OS and its OpenSSL version. Environments running an older OpenSSL (< 3.2) are not protected by Rust-level updates alone.

**Recommendation:** Install `cargo-audit` (`cargo install cargo-audit`) and run it as part of CI alongside `cargo deny check advisories`.

---

## 2. Autenticazione (orka-auth)

### JWT (`crates/orka-auth/src/jwt.rs`)

| Property | Status |
|---|---|
| Algorithm enforcement (HS256 / RS256) | ✅ Explicit algorithm in `Validation::new(Algorithm::HS256)` |
| `exp` validation | ✅ `validate_exp = true` |
| Issuer validation | ✅ `set_issuer(&[&issuer])` |
| Audience validation | ✅ `set_audience` when configured |
| Clock skew tolerance | ✅ 10 seconds (`leeway = 10`) |
| `alg: none` bypass | ✅ Not possible; library enforces configured algorithm |
| RSA PEM support | ✅ `with_rsa_pem` for RS256 |

**Finding — Minor (P3):** The `Claims` struct fields `_iss` and `_aud` are deserialized from the token but prefixed with underscores, implying they were originally intended for further manual re-validation that never happened. The actual issuer/audience enforcement is correctly done at the `Validation` level by `jsonwebtoken`, so this is cosmetic rather than exploitable. The underscored private fields may confuse future maintainers into thinking the validation is missing.

### API Key (`crates/orka-auth/src/api_key.rs`)

| Property | Status |
|---|---|
| Keys stored as hashes | ✅ SHA-256 |
| Salt per key | ❌ No salt |
| Timing-safe comparison | ⚠️ HashMap lookup — constant-time for the hash string comparison, but SHA-256 is fast |

**Finding — P2:** API key hashing uses plain SHA-256 without a per-key salt. If an attacker obtains the stored hash database (e.g., via Redis compromise), they could run an offline dictionary attack against short or low-entropy API keys. A slow KDF (bcrypt, Argon2id) or at minimum a random salt prepended to the hash would eliminate this risk.

### Middleware (`crates/orka-auth/src/middleware.rs`)

| Property | Status |
|---|---|
| Default: auth enabled | ✅ `enabled: true` in `Default` impl |
| Auth bypass when `enabled: false` | ✅ Explicit, by design (testing/internal use) |
| 401 response body | ✅ Generic `{"error": "unauthorized"}` — no detail leaked |
| Brute-force protection | ❌ None at middleware level |
| Rate limiting | ⚠️ Only at gateway level (`config.gateway.rate_limit`) |

**Finding — P2:** There is no request-level rate limiting or lockout on failed authentication attempts within `AuthMiddlewareConfig`. Brute-force of API keys (or password-spraying of JWT secrets) is only mitigated if the gateway-level rate limiter is configured — which is optional and off by default.

---

## 3. Gestione Secrets (orka-secrets)

### Encryption Implementation (`crates/orka-secrets/src/redis_secret.rs`)

| Property | Status |
|---|---|
| Algorithm | ✅ AES-256-GCM (authenticated encryption) |
| Key size enforcement | ✅ Exactly 32 bytes required |
| Nonce generation | ✅ `OsRng` per encryption — each ciphertext has a unique nonce |
| Nonce uniqueness | ✅ Random per call; tampering rejected via GCM auth tag |
| Tamper detection | ✅ GCM authentication tag; tested in `decrypt_tampered_ciphertext_fails` |

### Secret Loading (`crates/orka-secrets/src/lib.rs`)

| Property | Status |
|---|---|
| Key loaded from environment variable | ✅ `ORKA_SECRET_ENCRYPTION_KEY` (hex-encoded) |
| Key never in source / config files | ✅ Env-var only |
| Production guard | ✅ Hard-fail on startup if key unset and `ORKA_ENV=production` |
| Development fallback | ⚠️ Plaintext storage with `warn!()` log — acceptable for dev only |

**Finding — P3:** The development plaintext fallback is intentional and correctly guarded. However, the `ORKA_ENV` check is a simple string comparison (`eq_ignore_ascii_case("production")`). If the env var is not set at all (common in staging environments), the system silently stores secrets in plaintext. A more conservative approach would require an explicit `ORKA_ENV=development` opt-in rather than defaulting to plaintext.

**Finding — No hardcoded secrets found** in production code. Test-only constants (e.g., `[0xABu8; 32]` in unit tests) are appropriately scoped to `#[cfg(test)]`.

---

## 4. Sandboxing OS (orka-os)

### Privilege Check (`crates/orka-os/src/lib.rs`)

```rust
pub fn has_no_new_privileges() -> bool {
    unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) == 1 }
}
```

**Finding — P2:** `has_no_new_privileges()` only **reads** the `PR_GET_NO_NEW_PRIVS` flag — it does **not set** it. This is a probe function, not a hardening activation. If the calling process does not independently call `prctl(PR_SET_NO_NEW_PRIVS, 1, ...)` at startup, the flag will always be `0` and the check returns `false`. There is no evidence in the codebase that `PR_SET_NO_NEW_PRIVS` is ever called. This means SUID/SGID privilege escalation via spawned processes is not prevented at the OS level.

### Kernel-Level Isolation

| Mechanism | Status |
|---|---|
| `PR_SET_NO_NEW_PRIVS` | ❌ Not set by the process |
| seccomp BPF syscall filter | ❌ Not implemented |
| Linux namespaces (user/pid/net) | ❌ Not implemented |
| Landlock | ❌ Not implemented |
| cgroups resource limits | ❌ Not implemented |

**Finding — P1:** OS isolation relies entirely on the software-level `PermissionGuard` (allow/deny lists). If an agent is compromised or misconfigured, there is no kernel-level sandboxing to prevent privilege escalation or syscall abuse.

### PermissionGuard (`crates/orka-os/src/guard.rs`)

| Property | Status |
|---|---|
| Path canonicalization before allow/deny check | ✅ `path.canonicalize()` prevents symlink traversal |
| Path traversal via `..` | ✅ Resolved by `canonicalize()` before list check |
| Shell interpretation | ✅ `Command::new()` — no shell, args passed separately |
| Compound command splitting | ✅ `shell_words::split()` — POSIX quoting rules, no shell injection |
| `allowed_commands` default (empty list = all allowed) | ⚠️ Intentional but risk-significant |

**Finding — P2:** When `allowed_commands` is empty (the default), **all commands are permitted**. The comment in `check_command` acknowledges this as consistent design. However, in practice this means a newly deployed agent with default config has no command restrictions. The default should arguably be a restrictive mode (empty = deny all) rather than permissive (empty = allow all).

**Finding — P2:** The `sudo` path is hardcoded to `"sudo"` (resolved via `$PATH`). If an attacker can manipulate `$PATH`, they can redirect `sudo` execution to a malicious binary. Consider using the absolute path `/usr/bin/sudo`.

---

## 5. Superficie A2A v1.0 (`crates/orka-a2a/`)

### Exposed Endpoints

| Endpoint | Auth Required | Notes |
|---|---|---|
| `GET /.well-known/agent.json` | ❌ Always public | By A2A spec — intended |
| `POST /a2a` | ⚠️ Optional via `a2a.auth_enabled` | **Default: false** |
| SSE streaming (`message/stream`) | Same as `POST /a2a` | Inherits parent route auth |
| `tasks/subscribe` SSE | Same as `POST /a2a` | Inherits parent route auth |

**Finding — P1 (Critical):** `a2a.auth_enabled` defaults to `false` in `A2aConfig::default()`. This means any deployed Orka instance exposing the A2A endpoint accepts unauthenticated JSON-RPC calls by default. All task management operations (`message/send`, `tasks/cancel`, `tasks/list`, etc.) are open to the network.

### Input Validation

| Field | Validation |
|---|---|
| `method` in JSON-RPC | ✅ Method dispatch with `MethodNotFound` on unknown |
| `id`, `params` | ✅ Parsed from JSON; invalid structures return `InvalidRequest` error |
| `task_id` / `contextId` | ✅ Caller-supplied IDs accepted as-is; stored as Redis keys under `orka:a2a:*` namespace |
| `PushNotificationConfig.url` | ❌ **No URL validation** — any string accepted |

**Finding — P1 (SSRF):** `PushNotificationConfig.url` is stored and POSTed to by `WebhookDeliverer` without any URL scheme, host, or IP validation. An attacker can register a push notification config with:
- `http://169.254.169.254/latest/meta-data/` (AWS IMDSv1 SSRF)
- `http://10.0.0.1:6379/` (Redis SSRF)
- `file://` or other local schemes (rejected by `reqwest` but still a risk surface)
- Internal service addresses

Since `POST /a2a` is unauthenticated by default, this SSRF is exploitable without credentials.

### Task Enumeration / Tenant Isolation

**Finding — P2:** `tasks/list` returns ALL tasks across all callers with no per-user or per-tenant scoping. The `tenant` field in `JsonRpcRequest` is parsed and stored as task metadata but is never used as a filter or access control check. Any caller can enumerate every task on the server.

### SSE / Streaming

**Finding — P3:** The broadcast channel for `message/stream` and `tasks/subscribe` has a fixed capacity of 64 events. If a slow subscriber or lagging delivery worker falls behind, events are silently dropped (`BroadcastStream` maps `Lagged` to `None`). This is logged but not signaled to the client. Under heavy load a client may miss state transitions without any error indication.

---

## 6. Endpoint Research (`crates/orka-server/src/router/research.rs`)

### Authentication

**Finding — P1:** The research endpoints (`/api/v1/research/*`) are placed in the `api_routes` group and therefore DO receive the optional `AuthLayer`. However, the integration test helper `test_router_with_research()` sets `auth_layer: None` — tests pass without authentication. More critically, if the server is deployed without configuring an `AuthLayer`, all research endpoints are completely unauthenticated.

There is no "research requires auth" guard in code; auth is solely delegated to the presence or absence of `AuthLayer` at the router level.

### Input Validation

The `CreateResearchCampaign` request body includes fields that are passed directly to skill invocations:

| Field | Validated At HTTP Layer | Risk |
|---|---|---|
| `name` | ✅ Non-empty check | Low |
| `task` | ✅ Non-empty check | Low |
| `verification_command` | ⚠️ Non-empty only | **Command injection** (see below) |
| `repo_path` | ❌ None | **Path traversal** |
| `baseline_ref` | ❌ None | **Git ref injection** |
| `editable_paths` | ✅ Non-empty check only | **Path traversal** |

**Finding — P1 (Command Injection Risk):** `verification_command` is stored in the campaign and passed directly as the `command` argument to the `experiment_run` skill (which internally invokes `shell_exec`). The `PermissionGuard.check_command()` check only validates against the configured `allowed_commands` allowlist. If `allowed_commands` is empty (default), **any command** submitted by the API caller will be executed. An unauthenticated caller (when auth is not configured) can execute arbitrary commands on the server.

```rust
// research/service.rs ~line 437
("command".into(), serde_json::json!(campaign.verification_command)),
```

**Finding — P2 (Path Traversal):** `repo_path` is passed as `cwd` to skill invocations with no canonicalization or allow-list check at the research service layer. The `PermissionGuard.check_path()` would catch this IF called, but the research service invokes skills via `SkillInput` which creates a `SkillContext` with the path from `repo_path` directly — the guard check depends on the skill's own implementation, not the research layer.

**Finding — P2 (Git Ref Injection):** `baseline_ref` is passed as the `base` argument to `git_worktree_create`. A malicious value like `--upload-pack=malicious` or a ref with shell metacharacters could cause unexpected behavior if the underlying git invocation is not properly escaped.

### Error Information Disclosure

```rust
// research.rs line 35
e => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
```

**Finding — P2:** The `research_error` function returns `e.to_string()` directly in the HTTP response body for all non-NotFound/Conflict errors. This can leak internal details including file paths, Redis connection strings, and skill execution traces.

---

## 7. Injection e Input Validation — Analisi Trasversale

### Shell Injection

✅ **Not vulnerable** in the OS skills layer. `ShellExecSkill` uses `tokio::process::Command::new(command)` with arguments passed as separate items. No shell (`/bin/sh -c`) is invoked. Compound command strings are split via `shell_words::split` (POSIX quoting).

### Redis Key Injection

⚠️ **Low risk, not exploitable.** Task IDs from the caller are used as Redis keys under a fixed prefix (`orka:a2a:task:{id}`). Since Redis key injection doesn't escalate to code execution and the namespace prevents collisions with other data, the practical risk is limited to task ID collisions between callers.

### Path Traversal

⚠️ OS skills use `canonicalize()` before checking allow/deny lists — symlink traversal and `../` attacks are blocked.

**At the research layer, `repo_path` and `editable_paths` are not canonicalized** before being passed to skills. See Section 6.

### SQL / NoSQL Injection

✅ No SQL queries found. Redis commands use parameterized calls via the `redis` crate's typed API (no string interpolation into commands).

### Regex DoS (ReDoS)

⚠️ `campaign.metric.regex` is validated with `Regex::new()` at campaign creation time. The `regex` crate in Rust uses a linear-time engine that is immune to catastrophic backtracking. ✅

---

## 8. Gestione Errori — Information Disclosure

| Location | What is Leaked | Severity |
|---|---|---|
| `GET /health/ready` | Redis/Qdrant connection errors including URLs: `"error: Connection refused to redis://..."` | P2 |
| `research_error` (500 case) | `e.to_string()` — internal error messages, paths | P2 |
| A2A `A2aError::Internal(msg)` | Error message in `error.message` of JSON-RPC response | P2 |
| Management router | `format!("eval failed: {e}")` and similar in 500 bodies | P2 |
| Auth middleware 401 | Generic `{"error": "unauthorized"}` — ✅ no detail | ✅ Safe |

**Finding — P2 (Global):** Multiple endpoints return `e.to_string()` or `format!("... {e}")` in HTTP 500 response bodies. In production environments this can expose:
- Internal service hostnames and port numbers
- Redis/Qdrant connection strings (potentially including passwords embedded in URLs)
- File system paths
- Rust library version strings

The `/health/ready` endpoint is particularly sensitive: Redis errors include the full URL that was used to connect. If `ORKA_REDIS_URL` includes a password (`redis://:password@host:6379`), that password will appear in the unauthenticated `/health/ready` response.

---

## 9. Findings Riassuntivi

### P0 — Critico (Exploitabile senza autenticazione, impatto severo)

Nessuno identificato. La combinazione di P1 issues può però produrre impatto P0 in deployment reali.

---

### P1 — Alto

| ID | Titolo | Crate | Dettaglio |
|---|---|---|---|
| P1-01 | POST /a2a unauthenticated by default | `orka-a2a`, `orka-core/config` | `a2a.auth_enabled` defaults to `false`. Tutta la surface A2A è pubblica senza configurazione esplicita. |
| P1-02 | SSRF via push notification webhook URL | `orka-a2a` | `PushNotificationConfig.url` non è validato. Qualsiasi URL interno/IMDS può essere raggiunto dal server tramite `POST /a2a` (non autenticato per default). |
| P1-03 | Command injection via `verification_command` | `orka-research` | Il campo è passato direttamente allo skill `experiment_run`/`shell_exec`. Con `allowed_commands` vuoto (default), si esegue qualsiasi comando. |
| P1-04 | No kernel-level sandboxing per processi OS | `orka-os` | `PR_SET_NO_NEW_PRIVS` non è mai impostato. Nessun seccomp/landlock/namespace. Il sandboxing è puramente software. |

---

### P2 — Medio

| ID | Titolo | Crate | Dettaglio |
|---|---|---|---|
| P2-01 | SHA-256 non salted per API key hashing | `orka-auth` | Rainbow table attack possibile se il database degli hash viene compromesso. |
| P2-02 | No brute-force protection su autenticazione | `orka-auth` | Assenza di rate limiting e lockout a livello middleware. |
| P2-03 | `allowed_commands` vuoto = nessun filtro | `orka-os` | Il default permissivo non è sicuro in deploy pubblici. |
| P2-04 | `sudo` path non assoluto | `orka-os` | `"sudo"` risolto via `$PATH` è manipolabile. |
| P2-05 | `e.to_string()` in HTTP 500 responses | `orka-server` (research, management, dlq, schedules) | Leak di dettagli interni (path, URL, messaggi di errore interni). |
| P2-06 | `/health/ready` espone errori Redis/Qdrant | `orka-server/health.rs` | Connection errors (inclusi URL con password) in risposta HTTP pubblica. |
| P2-07 | `tasks/list` senza isolamento per tenant | `orka-a2a` | Qualsiasi caller può enumerare tutti i task del server. |
| P2-08 | `repo_path` senza canonicalizzazione | `orka-research` | Potenziale path traversal a livello research service. |
| P2-09 | `baseline_ref` senza validazione | `orka-research` | Possibile git ref injection nei worktree. |
| P2-10 | `ORKA_SECRET_ENCRYPTION_KEY` non richiesta in staging | `orka-secrets` | Solo `ORKA_ENV=production` forza la cifratura; staging può girare in plaintext. |

---

### P3 — Basso / Migliorativo

| ID | Titolo | Crate | Dettaglio |
|---|---|---|---|
| P3-01 | `_iss` / `_aud` in Claims struct fuorvianti | `orka-auth/jwt.rs` | Campi prefissati con `_` ma la validazione è corretta via `Validation`. Codice confuso. |
| P3-02 | `has_no_new_privileges()` non imposta il flag | `orka-os/lib.rs` | La funzione è un probe, non un'attivazione. Documentazione potenzialmente fuorviante. |
| P3-03 | SSE: eventi persi senza notifica client | `orka-a2a` | Broadcast channel capacity 64; lagging silently drops events. |
| P3-04 | `deny.toml` non ha `cargo audit` in CI | CI/infra | `cargo deny check advisories` non è configurato in `deny.toml` (solo `yanked`). |
| P3-05 | Research tests senza autenticazione | `orka-server/tests` | `test_router_with_research()` usa `auth_layer: None`, nascondendo potenziali regressioni auth. |

---

## Priorità di Remediation

1. **Immediato (P1-01 + P1-02):** Cambiare il default di `a2a.auth_enabled` a `true` e aggiungere validazione URL (schema allowlist `https://` only, blocco RFC1918) in `PushNotificationConfig`.
2. **Immediato (P1-03):** Aggiungere validazione di `verification_command` nel research service (allowlist di comandi, o rimozione del campo a favore di preset configurati).
3. **Breve termine (P2-05, P2-06):** Sostituire `e.to_string()` in 500 responses con messaggi generici; usare `tracing::error!` per i dettagli lato server.
4. **Breve termine (P2-01):** Aggiungere salt per-key nell'hashing delle API key (o migrare ad Argon2id).
5. **Medio termine (P1-04):** Valutare seccomp BPF o landlock per i processi spawned da `ShellExecSkill` in ambienti production.
6. **Medio termine (P2-03):** Invertire il default di `allowed_commands`: vuoto = deny all, non allow all.

