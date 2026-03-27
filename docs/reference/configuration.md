# Configuration Reference

Orka reads configuration from `orka.toml` plus `ORKA__...` environment overrides.

This page is a concise reference for the current top-level schema. For a
concrete end-to-end example, prefer the repository root
[`orka.toml`](../../orka.toml), which is intended to stay aligned with the
current parser.

## Environment Variables

| Variable | Description |
| --- | --- |
| `ORKA_CONFIG` | Path to the config file (default: `./orka.toml`) |
| `ORKA_ENV_FILE` | Optional `.env` file watched for secret rotation |
| `RUST_LOG` | Overrides tracing filter |
| `ANTHROPIC_API_KEY` | Fallback API key for Anthropic providers |
| `ANTHROPIC_AUTH_TOKEN` | Fallback bearer/setup-token for Anthropic providers |
| `OPENAI_API_KEY` | Fallback API key for OpenAI providers |
| `TAVILY_API_KEY` | Tavily search key |
| `BRAVE_API_KEY` | Brave search key |

Nested config values can be overridden with `ORKA__SECTION__KEY`. Arrays use numeric indexes, for example `ORKA__LLM__PROVIDERS__0__MODEL`.

## Top-Level Sections

| Section | Purpose |
| --- | --- |
| `config_version` | Schema version used by migrations and validation |
| `server` | Bind address for the HTTP server |
| `bus` | Message bus backend and polling behavior |
| `redis` | Redis connection |
| `logging` | Structured logging level and JSON mode |
| `workspace_dir` | Root directory used for workspace discovery |
| `workspaces` / `default_workspace` | Named runtime workspaces and default selection |
| `adapters` | Telegram, Discord, Slack, WhatsApp, custom adapter config |
| `worker` | Worker pool sizing and retry backoff |
| `memory` | Long-term key-value memory backend and capacity |
| `secrets` | Secret storage backend and encryption key source |
| `auth` | API-key / JWT auth config |
| `sandbox` | Process sandbox backend and resource limits |
| `plugins` | WASM plugin directory, capabilities, and per-plugin config |
| `soft_skills` | Discovery and loading rules for SKILL.md-based soft skills |
| `session` | Session TTL and backend behavior |
| `queue` | Queue retry policy |
| `llm` | Default LLM values plus provider list |
| `agents` | Agent definitions (array of tables) |
| `graph` | Graph topology for multi-agent execution |
| `tools` | Global allow/deny lists for skills |
| `observe` | Metrics and tracing backend |
| `audit` | Audit log output destination |
| `gateway` | Rate limiting and deduplication |
| `web` | Web search / read settings |
| `http` | HTTP client skill settings |
| `mcp` | External MCP server and client metadata |
| `guardrails` | Input/output validation rules |
| `os` | Local OS skill settings |
| `knowledge` | RAG and vector-store config |
| `scheduler` | Scheduled jobs |
| `prompts` | Prompt template directory and section ordering |
| `experience` | Reflection / distillation |
| `git` | Git skill guardrails and worktree policy |
| `a2a` | A2A discovery settings |
| `research` | Native research campaign feature gate and approval policy |
| `chart` | Chart-generation skill toggle |

## Key Fields

### `server`

| Key | Type | Notes |
| --- | --- | --- |
| `host` | `string` | Default `127.0.0.1` |
| `port` | `u16` | Default `8080` |

### `bus`

| Key | Type | Notes |
| --- | --- | --- |
| `backend` | `string` | Bus backend; defaults to Redis-backed runtime |
| `block_ms` | `u64` | `XREADGROUP BLOCK` timeout |
| `batch_size` | `usize` | Messages read per bus poll |
| `backoff_initial_secs` | `u64` | Reconnect backoff start |
| `backoff_max_secs` | `u64` | Reconnect backoff cap |

### `logging`

| Key | Type | Notes |
| --- | --- | --- |
| `level` | `string` | Structured log level |
| `json` | `bool` | Emit JSON logs |

### `workspace_dir`, `workspaces`, `default_workspace`

| Key | Type | Notes |
| --- | --- | --- |
| `workspace_dir` | `string` | Base directory for workspace discovery |
| `workspaces` | `array` | Optional named runtime workspaces |
| `default_workspace` | `string?` | Default workspace name when none is requested |

Each `[[workspaces]]` entry uses `name` and `dir`.

### `worker`

| Key | Type | Notes |
| --- | --- | --- |
| `concurrency` | `usize` | Number of concurrent workers |
| `retry_base_delay_ms` | `u64` | Base delay between retries |

### `adapters.telegram`

| Key | Type | Notes |
| --- | --- | --- |
| `bot_token_secret` | `string?` | Secret-store path for the bot token |
| `workspace` | `string?` | Workspace override |
| `mode` | `string?` | `polling` or `webhook` |
| `webhook_url` | `string?` | Required for webhook mode |
| `webhook_port` | `u16?` | Defaults to `8443` |
| `parse_mode` | `string?` | `HTML`, `MarkdownV2`, or `none` |
| `streaming` | `bool?` | Enable edit-message streaming |

`owner_id` and `allowed_users` are not part of the current config schema.

### `adapters.discord`

| Key | Type | Notes |
| --- | --- | --- |
| `bot_token_secret` | `string?` | Secret-store path for the bot token |
| `workspace` | `string?` | Workspace override |

### `adapters.slack`

| Key | Type | Notes |
| --- | --- | --- |
| `bot_token_secret` | `string?` | Secret-store path for the bot token |
| `signing_secret_path` | `string?` | Signing secret |
| `workspace` | `string?` | Workspace override |
| `port` | `u16` | Default `3000` |

### `adapters.whatsapp`

| Key | Type | Notes |
| --- | --- | --- |
| `access_token_secret` | `string?` | Secret-store path for Cloud API token |
| `app_secret_path` | `string?` | Optional app secret |
| `phone_number_id` | `string?` | WhatsApp phone number id |
| `business_account_id` | `string?` | Optional business account id |
| `workspace` | `string?` | Workspace override |
| `port` | `u16` | Default `3000` |
| `verify_token` | `string?` | Webhook verify token |

### `adapters.custom`

| Key | Type | Notes |
| --- | --- | --- |
| `host` | `string` | Default `127.0.0.1` |
| `port` | `u16` | Default `8081` |
| `webhook_path` | `string?` | Defaults to `/webhook` |
| `bearer_token_secret` | `string?` | Optional bearer token secret |
| `workspace` | `string?` | Workspace override |
| `headers` | `map<string,string>` | Extra response headers |

### `auth`

| Key | Type | Notes |
| --- | --- | --- |
| `api_keys` | `array` | API-key auth is active when this is non-empty |
| `jwt` | `table?` | JWT auth is active when this is set |
| `token_url` | `string?` | OAuth metadata |
| `auth_url` | `string?` | OAuth metadata |

There is no top-level `auth.enabled` field in the current schema.

### `sandbox`

| Key | Type | Notes |
| --- | --- | --- |
| `backend` | `string` | Sandbox backend |
| `allowed_paths` | `string[]` | Optional filesystem allowlist |
| `denied_paths` | `string[]` | Optional denylist |
| `limits.timeout_secs` | `u64` | Process timeout |
| `limits.max_memory_bytes` | `usize` | Memory cap |
| `limits.max_output_bytes` | `usize` | Output cap |
| `limits.max_open_files` | `usize?` | Optional FD limit |
| `limits.max_pids` | `usize` | Process-count limit |

### `plugins`

| Key | Type | Notes |
| --- | --- | --- |
| `dir` | `string?` | Directory containing WASM plugins |
| `capabilities.filesystem` | `bool|string[]` | Filesystem capability shorthand or explicit list |
| `capabilities.network` | `bool` | Allow outbound network |
| `capabilities.env` | `string[]` | Visible env vars |
| `plugins.<name>.enabled` | `bool` | Per-plugin toggle |
| `plugins.<name>.capabilities` | `table?` | Optional capability overrides |
| `plugins.<name>.config` | `table` | Plugin-specific passthrough config |

### `soft_skills`

| Key | Type | Notes |
| --- | --- | --- |
| `dir` | `string?` | Directory scanned for `SKILL.md` soft skills |
| `selection_mode` | `string` | `"all"` or `"keyword"` |

### `memory`

| Key | Type | Notes |
| --- | --- | --- |
| `backend` | `string` | `redis`, `memory`, or `auto` |
| `max_entries` | `usize` | Maximum stored entries |

### `secrets`

| Key | Type | Notes |
| --- | --- | --- |
| `backend` | `string` | `redis` or `file` |
| `file_path` | `string?` | Used by file-backed secrets |
| `encryption_key_path` | `string?` | Path to the master key |
| `encryption_key_env` | `string?` | Env var containing the master key |

The secret config also flattens Redis connection settings for the Redis-backed
backend.

### `session`

| Key | Type | Notes |
| --- | --- | --- |
| `ttl_secs` | `u64` | Session expiration in seconds |

### `queue`

| Key | Type | Notes |
| --- | --- | --- |
| `max_retries` | `u32` | Delivery retry limit before DLQ |

### `agents` (array of tables)

Each `[[agents]]` entry defines one agent in the execution graph.

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `id` | `string` | required | Unique agent identifier |
| `kind` | `string` | `"agent"` | Node behaviour: `"agent"`, `"router"`, `"fan_out"`, `"fan_in"` |
| `name` | `string` | `"Orka"` | Human-readable agent name |
| `system_prompt` | `string` | `""` | Optional static prompt |
| `model` | `string` | global default | LLM model identifier |
| `temperature` | `f32` | `0.7` | Sampling temperature |
| `max_tokens` | `u32` | `4096` | Response token cap |
| `thinking` | `string?` | none | Reasoning effort: `"low"`, `"medium"`, `"high"`, `"max"` |
| `max_turns` | `usize` | `10` | Maximum tool-use turns per agent run |
| `tool_result_max_chars` | `usize` | `8000` | Tool output truncation |
| `allowed_tools` | `string[]` | `[]` | Optional allowlist (empty = all tools) |
| `denied_tools` | `string[]` | `[]` | Optional denylist (takes precedence) |
| `history_filter` | `string` | `"full"` | Handoff history strategy: `"full"`, `"last_n"`, `"none"` |
| `history_filter_n` | `usize?` | none | Messages to keep when `history_filter = "last_n"` |
| `planning_mode` | `string` | `"none"` | `"none"`, `"adaptive"` (LLM-driven plan tools), `"always"` (eager plan before first iteration) |
| `history_strategy` | `string` | `"truncate"` | `"truncate"`, `"summarize"` (LLM summary of dropped turns), `"rolling_window:<n>"` (keep last *n* turns) |
| `interrupt_before_tools` | `string[]` | `[]` | Tool names that pause execution for human approval (HITL); resume via `POST /api/v1/runs/{run_id}/approve` |
| `skill_timeout_secs` | `u64` | `120` | Per-skill execution timeout; skills exceeding this are cancelled |
| `max_concurrent_skills` | `usize?` | `null` | Reserved for future use |

**Node kinds:**
- `agent` — full LLM tool loop; can hand off to other agents via `transfer_to_agent` / `delegate_to_agent` (auto-injected)
- `router` — evaluates outgoing edge conditions without calling the LLM (use a cheap model)
- `fan_out` — dispatches to **all** successors in parallel (requires ≥ 2 outgoing edges)
- `fan_in` — waits for predecessors, then synthesizes results via LLM

The legacy `[agent]` single-table form is automatically promoted to `[[agents]]` + `[graph]` by the v4→v5 migration on first boot. Use `orka config migrate` to persist the conversion to disk.

### `graph`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `entry` | `string?` | first agent | Explicit entry-point agent ID |
| `execution_mode` | `string` | `"sequential"` | Graph execution mode |
| `max_hops` | `usize` | `10` | Max agent transitions per run |
| `edges` | `EdgeDef[]` | `[]` | Edge list connecting agents |
| `reducers` | `map<string,string>` | `{}` | State-slot merge strategies for fan-out aggregation; values: `"append"`, `"sum"`, `"max"`, `"min"`, `"merge_object"`, `"last_write_wins"` |

Each `[[graph.edges]]` entry: `from` (string), `to` (string), `condition` (string, optional), `weight` (f32, default `1.0`).

Condition syntax: `"always"`, `"output_contains:<text>"`, or `"state_match:<key>=<value>"`.

**Handoff targets** are auto-derived from outgoing edges for `agent`-kind nodes — no manual configuration needed. `router`, `fan_out`, and `fan_in` nodes use structural routing only.

### `llm`

| Key | Type | Notes |
| --- | --- | --- |
| `default_model` | `string` | Workspace-wide model default |
| `default_temperature` | `f32` | Default temperature |
| `default_max_tokens` | `u32` | Default token cap |
| `providers` | `array` | Provider configs |

Each `[[llm.providers]]` supports:

| Key | Type | Notes |
| --- | --- | --- |
| `name` | `string` | Unique provider name |
| `provider` | `string` | `anthropic`, `openai`, `ollama`, etc. |
| `auth_kind` | `enum` | `auto`, `api_key`, `auth_token`, `subscription`, `cli` |
| `base_url` | `string?` | Optional override |
| `model` | `string?` | Default model for that provider |
| `api_key` | `string?` | Inline API key |
| `api_key_env` | `string?` | Env var containing API key |
| `api_key_secret` | `string?` | Secret-store path |
| `auth_token` | `string?` | Inline bearer/auth token |
| `auth_token_env` | `string?` | Env var containing bearer/auth token |
| `auth_token_secret` | `string?` | Secret-store path for bearer/auth token |
| `temperature` | `f32?` | Provider default |
| `max_tokens` | `u32?` | Provider default |
| `top_p` | `f32?` | Provider default |
| `timeout_secs` | `u64?` | Per-provider timeout |
| `max_retries` | `u32?` | Per-provider retries |

Anthropic auth behavior:
- `auth_kind = "api_key"` uses `ANTHROPIC_API_KEY` / `api_key_*`.
- `auth_kind = "auth_token"` or `auth_kind = "subscription"` uses `ANTHROPIC_AUTH_TOKEN` / `auth_token_*` first, then legacy `api_key_*` for backward compatibility.
- `auth_kind = "auto"` keeps backward-compatible inference and may promote legacy Anthropic token shapes to bearer auth.
- `auth_kind = "cli"` is reserved for CLI-backed integration paths; delegated coding still belongs under `os.coding.providers.claude_code`.

The current schema does not define `llm.timeout_secs`, `llm.max_tokens`, `llm.api_version`, or provider `prefixes`.

### `tools`

| Key | Type | Notes |
| --- | --- | --- |
| `allow` | `string[]` | Global allowlist |
| `deny` | `string[]` | Global denylist |
| `config` | `map<string,json>` | Tool-specific config |

### `observe`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable metrics/tracing |
| `backend` | `string` | `stdout`, `prometheus`, or `otlp` |
| `otlp_endpoint` | `string?` | Collector endpoint |
| `batch_size` | `usize` | Metrics batch size |
| `flush_interval_ms` | `u64` | Flush cadence |
| `service_name` | `string` | Telemetry service name |
| `service_version` | `string` | Telemetry service version |

### `audit`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable audit logging |
| `output` | `string` | `stdout`, `file`, or `redis` |
| `path` | `path?` | File destination when `output = "file"` |
| `redis_key` | `string?` | Stream key when `output = "redis"` |

### `gateway`

| Key | Type | Notes |
| --- | --- | --- |
| `rate_limit` | `u32` | Requests per minute per session (`0` = unlimited) |
| `dedup_ttl_secs` | `u64` | Duplicate-detection window |
| `dedup_enabled` | `bool` | Toggle request deduplication |

### `web`

| Key | Type | Notes |
| --- | --- | --- |
| `search_provider` | `string` | `none`, `tavily`, `brave`, `searxng` |
| `api_key` | `string?` | Inline API key |
| `api_key_env` | `string?` | Env var containing API key |
| `searxng_base_url` | `string?` | Needed for `searxng` |
| `max_results` | `usize` | Search result cap |
| `max_read_chars` | `usize` | Read cap |
| `max_content_chars` | `usize` | Extracted-content cap |
| `cache_ttl_secs` | `u64` | Cache TTL |
| `read_timeout_secs` | `u64` | HTTP read timeout |
| `user_agent` | `string?` | Optional UA override |

### `http`

| Key | Type | Notes |
| --- | --- | --- |
| `timeout_secs` | `u64` | Client timeout |
| `max_redirects` | `usize` | Redirect limit |
| `user_agent` | `string?` | Optional UA override |
| `default_headers` | `[(string,string)]` | Default outbound headers |
| `webhooks` | `array` | Optional webhook definitions |

There is no `http.enabled` field in the current schema.

### `mcp`

| Key | Type | Notes |
| --- | --- | --- |
| `servers` | `array` | External MCP server definitions |
| `client.name` | `string` | MCP client display name |
| `client.version` | `string` | MCP client version |

Each `[[mcp.servers]]` entry supports `name`, `transport`, `command`, `args`,
`env`, `url`, `working_dir`, and optional OAuth settings under `auth`.

### `guardrails`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable guardrail pipeline |
| `input.blocked_keywords` | `string[]` | Input denylist |
| `input.blocked_patterns` | `string[]` | Input regex denylist |
| `input.redact_patterns` | `array` | Input redaction patterns |
| `input.max_length` | `usize?` | Max input length |
| `input.llm_moderation` | `table` | LLM moderation settings |
| `output.*` | same | Same structure for outbound validation |

### `os`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable OS skills |
| `permission_level` | `string` | `read-only`, `interact`, `write`, `execute`, `admin` |
| `allowed_paths` | `string[]` | Filesystem allowlist |
| `denied_paths` | `string[]` | Filesystem denylist |
| `allowed_shell_commands` | `string[]` | Shell allowlist |
| `coding` | `table` | Coding delegation routing policy |

#### `os.coding`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable the coding delegation subsystem |
| `default_provider` | `string` | `auto`, `claude_code`, `codex`, or `opencode` |
| `selection_policy` | `string` | `availability`, `prefer_claude`, `prefer_codex`, or `prefer_opencode` |
| `inject_workspace_context` | `bool` | Inject workspace cwd into delegated prompts |
| `require_verification` | `bool` | Require `verification` for delegated tasks |
| `allow_working_dir_override` | `bool` | Allow runtime `working_dir` overrides |
| `providers` | `table` | Nested backend configuration |

#### `os.coding.providers.claude_code`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable Claude Code integration |
| `executable_path` | `path?` | Optional explicit binary path |
| `model` | `string?` | Optional model override |
| `max_turns` | `u32?` | Optional Claude agent turn cap |
| `timeout_secs` | `u64` | Execution timeout |
| `append_system_prompt` | `string?` | Extra system prompt appended to the run |
| `allowed_tools` | `string[]` | Optional Claude tool allowlist |
| `allow_file_modifications` | `bool` | Allow edits |
| `allow_command_execution` | `bool` | Allow commands |

#### `os.coding.providers.codex`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable Codex integration |
| `executable_path` | `path?` | Optional explicit binary path |
| `model` | `string?` | Optional model override |
| `timeout_secs` | `u64` | Execution timeout |
| `sandbox_mode` | `string?` | `read-only`, `workspace-write`, or `danger-full-access` |
| `approval_policy` | `string?` | `untrusted`, `on-failure`, `on-request`, or `never` |
| `allow_file_modifications` | `bool` | Allow edits |
| `allow_command_execution` | `bool` | Allow commands |

#### `os.coding.providers.opencode`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable `OpenCode` integration |
| `executable_path` | `path?` | Optional explicit binary path |
| `model` | `string?` | Model in `provider/model` format (e.g. `anthropic/claude-sonnet-4-6`) |
| `agent` | `string?` | Agent name passed via `--agent` |
| `variant` | `string?` | Reasoning effort variant (`high`, `max`, `minimal`) |
| `timeout_secs` | `u64` | Execution timeout |
| `allow_file_modifications` | `bool` | Allow edits |
| `allow_command_execution` | `bool` | Allow commands |

#### `os.sudo`

| Key | Type | Notes |
| --- | --- | --- |
| `allowed` | `bool` | Enable sudo-capable skills |
| `allowed_commands` | `string[]` | Allowed sudo command prefixes |
| `password_required` | `bool` | Whether sudo expects a password |

The current schema does not define `require_confirmation`, `confirmation_timeout_secs`, or `sudo_path`.

### `knowledge`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable RAG |
| `vector_store.backend` | `enum` | Vector backend |
| `vector_store.url` | `string?` | Qdrant URL |
| `vector_store.collection_name` | `string` | Collection name |
| `vector_store.dimension` | `usize` | Embedding dimension |
| `vector_store.distance_metric` | `string` | `cosine`, `euclidean`, or `dot` |
| `embeddings.provider` | `enum` | `local`, `openai`, `anthropic`, `custom` |
| `embeddings.model` | `string` | Embedding model |
| `embeddings.api_key` | `string?` | Optional inline key |
| `embeddings.batch_size` | `usize` | Embedding batch size |
| `chunking.chunk_size` | `usize` | Chunk size |
| `chunking.chunk_overlap` | `usize` | Chunk overlap |
| `retrieval.top_k` | `usize` | Retrieval fan-out |
| `retrieval.score_threshold` | `f32` | Minimum similarity |
| `retrieval.rerank` | `bool` | Enable reranking |

### `scheduler`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable scheduler |
| `jobs` | `array` | Scheduled job list |

Each job supports `name`, `schedule`, `command`, `workspace`, and `enabled`.

### `prompts`

| Key | Type | Notes |
| --- | --- | --- |
| `templates_dir` | `string` | Custom template directory |
| `hot_reload` | `bool` | Watch templates for changes |
| `section_order` | `string[]` | Default system-prompt section order |
| `section_separator` | `string` | Separator inserted between sections |
| `max_principles` | `usize` | Maximum learned principles injected |

### `experience`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable self-learning |
| `max_principles` | `usize` | Max injected learned principles |
| `min_relevance_score` | `f32` | Principle relevance cutoff |
| `reflect_on` | `string` | `failures`, `all`, or `sampled` |
| `sample_rate` | `f64` | Reflection sampling when `sampled` |
| `principles_collection` | `string` | Vector collection for principles |
| `trajectories_collection` | `string` | Vector collection for trajectories |
| `reflection_model` | `string?` | Optional LLM override |
| `reflection_max_tokens` | `u32` | Token budget for reflection |
| `distillation_batch_size` | `usize` | Batch size per distillation run |
| `dedup_threshold` | `f32` | Principle dedup threshold |
| `distillation_interval_secs` | `u64` | Background distillation cadence |

### `git`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable git skills |
| `protected_branches` | `string[]` | Branches agents must not push to directly |
| `allow_force_push` | `bool` | Permit force-push |
| `require_conventional_commits` | `bool` | Enforce Conventional Commits |
| `sign_commits` | `bool` | Pass `-S` on commit |
| `secret_patterns` | `string[]` | Files that must not be committed |
| `allowed_remotes` | `string[]` | Remote allowlist |
| `max_diff_lines` | `usize` | Diff output cap |
| `max_log_entries` | `usize` | Log output cap |
| `command_timeout_secs` | `u64` | Git command timeout |
| `authorship` | `table` | Commit attribution policy |
| `worktree` | `table` | Worktree creation policy |

### `a2a`

| Key | Type | Notes |
| --- | --- | --- |
| `discovery_enabled` | `bool` | Enable discovery loop |
| `discovery_interval_secs` | `u64` | Discovery interval |
| `known_agents` | `string[]` | Seed endpoints |
| `auth_enabled` | `bool` | Mount `POST /a2a` behind auth |
| `store_backend` | `string` | `memory` or `redis` |

There is no `a2a.enabled` or `a2a.url` field in the current schema.

### `research`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable the native research subsystem |
| `require_promotion_approval` | `bool` | Require explicit approval before promotion |
| `protected_target_branches` | `string[]` | Branch globs that always require approval |

### `chart`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable chart-generation skills |

## Notes

- In high-risk sections such as `agent`, `auth`, `llm`, `tools`, `observe`, `os`, `http`, and `scheduler`, legacy or unknown active keys now fail validation instead of being ignored silently.
- The repository root [`orka.toml`](../../orka.toml) is the canonical sample configuration.
- The `research` section exists in the config schema, but the current public CLI
  reference does not expose an `orka research ...` command.
