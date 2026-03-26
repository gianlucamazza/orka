# Configuration Reference

Orka reads configuration from `orka.toml` plus `ORKA__...` environment overrides.

## Environment Variables

| Variable | Description |
| --- | --- |
| `ORKA_CONFIG` | Path to the config file (default: `./orka.toml`) |
| `ORKA_ENV_FILE` | Optional `.env` file watched for secret rotation |
| `RUST_LOG` | Overrides tracing filter |
| `ANTHROPIC_API_KEY` | Fallback API key for Anthropic providers |
| `OPENAI_API_KEY` | Fallback API key for OpenAI providers |
| `TAVILY_API_KEY` | Tavily search key |
| `BRAVE_API_KEY` | Brave search key |

Nested config values can be overridden with `ORKA__SECTION__KEY`. Arrays use numeric indexes, for example `ORKA__LLM__PROVIDERS__0__MODEL`.

## Top-Level Sections

| Section | Purpose |
| --- | --- |
| `server` | Bind address for the HTTP server |
| `redis` | Redis connection |
| `adapters` | Telegram, Discord, Slack, WhatsApp, custom adapter config |
| `worker` | Worker pool sizing and retry backoff |
| `auth` | API-key / JWT auth config |
| `llm` | Default LLM values plus provider list |
| `agents` | Agent definitions (array of tables) |
| `graph` | Graph topology for multi-agent execution |
| `tools` | Global allow/deny lists for skills |
| `web` | Web search / read settings |
| `http` | HTTP client skill settings |
| `os` | Local OS skill settings |
| `knowledge` | RAG and vector-store config |
| `scheduler` | Scheduled jobs |
| `experience` | Reflection / distillation |
| `mcp` | External MCP servers |
| `a2a` | A2A discovery settings |

## Key Fields

### `server`

| Key | Type | Notes |
| --- | --- | --- |
| `host` | `string` | Default `127.0.0.1` |
| `port` | `u16` | Default `8080` |

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
| `max_iterations` | `usize` | `10` | Agent LLM loop cap |
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
| `base_url` | `string?` | Optional override |
| `model` | `string?` | Default model for that provider |
| `api_key` | `string?` | Inline API key |
| `api_key_env` | `string?` | Env var containing API key |
| `api_key_secret` | `string?` | Secret-store path |
| `temperature` | `f32?` | Provider default |
| `max_tokens` | `u32?` | Provider default |
| `top_p` | `f32?` | Provider default |
| `timeout_secs` | `u64?` | Per-provider timeout |
| `max_retries` | `u32?` | Per-provider retries |

The current schema does not define `llm.timeout_secs`, `llm.max_tokens`, `llm.api_version`, or provider `prefixes`.

### `tools`

| Key | Type | Notes |
| --- | --- | --- |
| `allow` | `string[]` | Global allowlist |
| `deny` | `string[]` | Global denylist |
| `config` | `map<string,json>` | Tool-specific config |

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
| `default_provider` | `string` | `auto`, `claude_code`, or `codex` |
| `selection_policy` | `string` | `availability`, `prefer_claude`, or `prefer_codex` |
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
| `embeddings.provider` | `enum` | `local`, `openai`, `anthropic`, `custom` |
| `embeddings.model` | `string` | Embedding model |
| `embeddings.api_key` | `string?` | Optional inline key |
| `embeddings.batch_size` | `usize` | Embedding batch size |

### `scheduler`

| Key | Type | Notes |
| --- | --- | --- |
| `enabled` | `bool` | Enable scheduler |
| `jobs` | `array` | Scheduled job list |

Each job supports `name`, `schedule`, `command`, `workspace`, and `enabled`.

### `a2a`

| Key | Type | Notes |
| --- | --- | --- |
| `discovery_enabled` | `bool` | Enable discovery loop |
| `discovery_interval_secs` | `u64` | Discovery interval |
| `known_agents` | `string[]` | Seed endpoints |

There is no `a2a.enabled` or `a2a.url` field in the current schema.

## Notes

- In high-risk sections such as `agent`, `auth`, `llm`, `tools`, `observe`, `os`, `http`, and `scheduler`, legacy or unknown active keys now fail validation instead of being ignored silently.
- For a concrete example, prefer the repository root [`orka.toml`](/home/gianluca/Workspace/orka/orka.toml), which should stay aligned with the current parser.
