# Configuration Reference

Orka reads configuration from `orka.toml` and `ORKA_*` environment variables.

## Environment Variables

| Variable                     | Description                                             |
| ---------------------------- | ------------------------------------------------------- |
| `ORKA_CONFIG`                | Path to config file (default: `./orka.toml`)            |
| `ORKA_ENV_FILE`              | Path to `.env` file for hot-reload                      |
| `ORKA_ENV` / `APP_ENV`       | `production` requires encryption key for secrets        |
| `ORKA_SECRET_ENCRYPTION_KEY` | 32-byte hex key for AES-256-GCM secret encryption       |
| `ORKA_HOST_HOSTNAME`         | Override hostname in system info                        |
| `ORKA_SERVER_URL`            | CLI: server endpoint (default `http://127.0.0.1:8080`)  |
| `ORKA_ADAPTER_URL`           | CLI: adapter endpoint (default `http://127.0.0.1:8081`) |
| `ORKA_API_KEY`               | CLI: API key for authenticated requests                 |
| `ANTHROPIC_API_KEY`          | Anthropic provider fallback                             |
| `OPENAI_API_KEY`             | OpenAI provider fallback                                |
| `TAVILY_API_KEY`             | Tavily web search key                                   |
| `BRAVE_API_KEY`              | Brave web search key                                    |
| `RUST_LOG`                   | Overrides `logging.level` via tracing `EnvFilter`       |
| `ORKA_GIT_SHA`               | Git SHA embedded at build time                          |
| `ORKA_BUILD_DATE`            | Build date embedded at build time                       |
| `ORKA_NO_UPDATE_CHECK`       | Disable automatic update check on CLI startup           |

Config fields can also be overridden via `ORKA__<SECTION>__<KEY>` (e.g., `ORKA__REDIS__URL`).

## `orka.toml` Options

| Section                   | Key                       | Default                  | Description                                                             |
| ------------------------- | ------------------------- | ------------------------ | ----------------------------------------------------------------------- |
| `server`                  | `host`                    | `127.0.0.1`              | Health endpoint bind address                                            |
| `server`                  | `port`                    | `8080`                   | Health endpoint port                                                    |
| `redis`                   | `url`                     | `redis://127.0.0.1:6379` | Redis connection URL                                                    |
| `worker`                  | `concurrency`             | `4`                      | Number of concurrent workers                                            |
| `session`                 | `ttl_secs`                | `86400`                  | Session TTL in seconds (24h)                                            |
| `queue`                   | `max_retries`             | `3`                      | Max retries before dead-letter                                          |
| `adapters.custom`         | `host`                    | `127.0.0.1`              | Custom adapter bind address                                             |
| `adapters.custom`         | `port`                    | `8081`                   | Custom adapter port                                                     |
| `adapters.telegram`       | `bot_token_secret`        | —                        | Secret path for bot token                                               |
| `adapters.telegram`       | `mode`                    | `polling`                | `polling` or `webhook`                                                  |
| `adapters.telegram`       | `parse_mode`              | `HTML`                   | Outbound text format                                                    |
| `adapters.telegram`       | `webhook_url`             | —                        | Public URL for webhook mode                                             |
| `adapters.telegram`       | `webhook_port`            | `8443`                   | Local port for webhook listener                                         |
| `auth`                    | `enabled`                 | `false`                  | Enable API key authentication                                           |
| `sandbox`                 | `backend`                 | `process`                | Sandbox backend (`process` or `wasm`)                                   |
| `logging`                 | `level`                   | `info`                   | Log level                                                               |
| `logging`                 | `json`                    | `false`                  | JSON log format                                                         |
| `agent`                   | `id`                      | `orka-default`           | Agent identifier                                                        |
| `agent`                   | `max_iterations`          | `10`                     | Max agentic loop iterations per turn                                    |
| `agent`                   | `heartbeat_interval_secs` | —                        | Streaming heartbeat interval (optional)                                 |
| `llm`                     | `timeout_secs`            | `30`                     | LLM request timeout                                                     |
| `llm`                     | `max_tokens`              | `8192`                   | Default max output tokens                                               |
| `llm.providers`           | `name`                    | —                        | Provider name (array of provider configs)                               |
| `knowledge`               | `enabled`                 | `false`                  | Enable RAG/knowledge base                                               |
| `knowledge.vector_store`  | `provider`                | `qdrant`                 | Vector store backend                                                    |
| `knowledge.vector_store`  | `url`                     | `http://localhost:6334`  | Qdrant endpoint                                                         |
| `scheduler`               | `enabled`                 | `false`                  | Enable cron scheduler                                                   |
| `scheduler`               | `poll_interval_secs`      | `5`                      | Scheduler polling interval                                              |
| `web`                     | `search_provider`         | `none`                   | Web search backend (`tavily`, `brave`, `searxng`, or `none`)            |
| `os`                      | `enabled`                 | `false`                  | Enable OS integration skills                                            |
| `os`                      | `permission_level`        | `read-only`              | OS skill permission level                                               |
| `http`                    | `enabled`                 | `false`                  | Enable HTTP request skill                                               |
| `plugins`                 | `dir`                     | —                        | Directory for WASM plugin files (optional)                              |
| `guardrails`              | `blocked_keywords`        | `[]`                     | Keywords that trigger message blocking                                  |
| `guardrails`              | `pii_filter`              | `false`                  | Enable PII redaction                                                    |
| `mcp.servers`             | `name`                    | —                        | MCP server name (array of server configs)                               |
| `mcp.servers`             | `command`                 | —                        | Command to launch MCP server                                            |
| `mcp.serve`               | `enabled`                 | `false`                  | Expose Orka as an MCP server                                            |
| `mcp.serve`               | `transport`               | `stdio`                  | `stdio` or `sse`                                                        |
| `mcp.servers[].transport` | `type`                    | `stdio`                  | `stdio` or `streamable_http` (MCP spec 2025-03-26)                      |
| `mcp.servers[].transport` | `url`                     | —                        | HTTP endpoint for `streamable_http` transport                           |
| `mcp.servers[].transport` | `auth.token_url`          | —                        | OAuth 2.1 token endpoint (optional)                                     |
| `mcp.servers[].transport` | `auth.client_id`          | —                        | OAuth 2.1 client ID                                                     |
| `mcp.servers[].transport` | `auth.client_secret_env`  | —                        | Env var holding the OAuth client secret                                 |
| `mcp.servers[].transport` | `auth.scopes`             | `[]`                     | OAuth scopes to request                                                 |
| `bus`                     | `backend`                 | `redis`                  | Message bus backend (`redis`, `nats`, or `memory`)                      |
| `bus`                     | `block_ms`                | `5000`                   | XREADGROUP BLOCK timeout (ms)                                           |
| `bus`                     | `batch_size`              | `10`                     | Messages per read batch                                                 |
| `memory`                  | `backend`                 | `auto`                   | `redis`, `memory`, or `auto`                                            |
| `session`                 | `backend`                 | `auto`                   | `redis`, `memory`, or `auto`                                            |
| `queue`                   | `backend`                 | `auto`                   | `redis`, `memory`, or `auto`                                            |
| `observe`                 | `backend`                 | `log`                    | `log`, `redis`, or `otel`                                               |
| `agent`                   | `max_history_entries`     | `50`                     | Max conversation turns kept in context                                  |
| `agent`                   | `skill_timeout_secs`      | `120`                    | Per-skill execution timeout                                             |
| `agent`                   | `temperature`             | —                        | LLM sampling temperature (0.0–2.0)                                      |
| `agent`                   | `thinking_budget_tokens`  | —                        | Anthropic extended thinking budget                                      |
| `agent`                   | `reasoning_effort`        | —                        | OpenAI o-series: `low`, `medium`, `high`                                |
| `experience`              | `enabled`                 | `false`                  | Enable self-learning experience loop                                    |
| `experience`              | `reflect_on`              | `failures`               | `failures`, `all`, or `sampled`                                         |
| `experience`              | `max_principles`          | `5`                      | Max principles injected into system prompt                              |
| `a2a`                     | `enabled`                 | `false`                  | Enable Agent-to-Agent protocol                                          |
| `os`                      | `sensitive_env_patterns`  | glob list                | Env var patterns redacted from tool output                              |
| `os`                      | `allowed_commands`        | `[]`                     | Explicit command allow-list for OS skills                               |
| `os`                      | `allowed_paths`           | `["/home", "/tmp"]`      | Filesystem access allow-list                                            |
| `os`                      | `blocked_paths`           | (see orka.toml)          | Filesystem access deny-list                                             |
| `os`                      | `blocked_commands`        | (see orka.toml)          | Dangerous command deny-list                                             |
| `os`                      | `max_file_size_bytes`     | `10485760`               | Max file size for reads (10 MB)                                         |
| `os`                      | `shell_timeout_secs`      | `30`                     | Shell command timeout                                                   |
| `os.sudo`                 | `enabled`                 | `false`                  | Enable sudo operations                                                  |
| `os.sudo`                 | `require_confirmation`    | `true`                   | Require user confirmation for sudo                                      |
| `os.claude_code`          | `enabled`                 | `"auto"`                 | Auto-detect claude CLI on PATH (`"true"`/`"false"` to override)         |
| `os.claude_code`          | `model`                   | —                        | Claude model override (e.g. `claude-sonnet-4-6`)                        |
| `os.claude_code`          | `max_turns`               | —                        | Max agentic turns per task                                              |
| `os.claude_code`          | `timeout_secs`            | `300`                    | Subprocess timeout in seconds                                           |
| `os.claude_code`          | `working_dir`             | —                        | Working directory for the subprocess                                    |
| `os.claude_code`          | `system_prompt`           | —                        | Instructions appended via `--append-system-prompt`                      |
| `os.claude_code`          | `allowed_tools`           | `[]`                     | Tool allowlist for Claude Code (`--allowedTools`); empty = unrestricted |
| `os.claude_code`          | `inject_context`          | `true`                   | Auto-inject workspace info (cwd) into the task prompt                   |
| `gateway`                 | `rate_limit`              | `60`                     | Max messages per 60s window per session                                 |
| `gateway`                 | `dedup_ttl_secs`          | `3600`                   | Duplicate message detection window                                      |
| `sandbox.limits`          | `timeout_secs`            | `30`                     | Execution timeout                                                       |
| `sandbox.limits`          | `max_memory_bytes`        | `67108864`               | Memory limit (64 MB)                                                    |
| `sandbox.limits`          | `max_output_bytes`        | `1048576`                | Output limit (1 MB)                                                     |
| `soft_skills`             | `dir`                     | —                        | Directory of SKILL.md soft-skill subdirectories                         |
| `audit`                   | `enabled`                 | `false`                  | Enable skill invocation audit log                                       |
| `audit`                   | `output`                  | `file`                   | Audit backend: `file` (JSONL) or `redis`                                |
| `audit`                   | `path`                    | `orka-audit.jsonl`       | Output path for file-based audit log                                    |
| `tools`                   | `disabled`                | `[]`                     | Skill names to disable                                                  |
| `secrets`                 | `encryption_key_env`      | —                        | Env var name for encryption key                                         |
| `auth`                    | `api_key_header`          | `X-Api-Key`              | Header name for API key auth                                            |
| `worker`                  | `retry_base_delay_ms`     | `5000`                   | Base delay for exponential backoff                                      |
| `memory`                  | `max_entries`             | `10000`                  | Max key-value memory entries                                            |
| `observe`                 | `batch_size`              | `50`                     | Event batch size before flush                                           |
| `observe`                 | `flush_interval_ms`       | `100`                    | Flush interval (ms)                                                     |
| `knowledge.embeddings`    | `provider`                | `local`                  | Embedding provider                                                      |
| `knowledge.embeddings`    | `model`                   | `BAAI/bge-small-en-v1.5` | Embedding model                                                         |
| `knowledge.chunking`      | `chunk_size`              | `1000`                   | Characters per chunk                                                    |
| `knowledge.chunking`      | `chunk_overlap`           | `200`                    | Overlap between chunks                                                  |
| `scheduler`               | `max_concurrent`          | `4`                      | Max concurrent scheduled tasks                                          |
| `llm`                     | `model`                   | `claude-sonnet-4-6`      | Global default model                                                    |
| `llm`                     | `max_retries`             | `2`                      | LLM request retries                                                     |
| `llm`                     | `context_window_tokens`   | `1000000`                | Context window size                                                     |
| `web`                     | `max_read_chars`          | `20000`                  | Max chars per page read                                                 |
| `web`                     | `max_content_chars`       | `8000`                   | Truncated content limit per page                                        |
| `web`                     | `cache_ttl_secs`          | `3600`                   | Search cache TTL                                                        |
| `http`                    | `max_response_bytes`      | `1048576`                | Max HTTP response size (1 MB)                                           |
| `http`                    | `default_timeout_secs`    | `30`                     | HTTP request timeout                                                    |
| `http`                    | `blocked_domains`         | `["169.254.169.254"]`    | SSRF protection deny-list                                               |
| `agent`                   | `max_tool_result_chars`   | `50000`                  | Truncation limit for tool output                                        |
| `agent`                   | `max_tool_retries`        | `2`                      | Retries before self-correction hint                                     |

## Adapter Configuration

Orka supports multiple messaging adapters. Below are configuration examples for each.

### Telegram Adapter

```toml
[adapters.telegram]
bot_token_secret = "telegram_token"  # Secret name from secret store
mode = "polling"                      # "polling" or "webhook"
parse_mode = "HTML"                   # "HTML" or "MarkdownV2"

# Webhook mode (production)
# webhook_url = "https://your-domain.com/webhook/telegram"
# webhook_port = 8443
```

**Setup:**
1. Create bot via @BotFather in Telegram
2. Store token: `orka secret set telegram_token "YOUR_TOKEN"`
3. Configure adapter in `orka.toml`

### Discord Adapter

```toml
[adapters.discord]
bot_token_secret = "discord_token"
application_id = "your_app_id"  # Optional, for slash commands
```

### Slack Adapter

```toml
[adapters.slack]
bot_token_secret = "slack_bot_token"
listen_port = 3000
```

### WhatsApp Adapter

```toml
[adapters.whatsapp]
access_token_secret = "whatsapp_token"
phone_number_id = "your_phone_id"
verify_token = "webhook_verify_token"
listen_port = 3001
```

### Custom HTTP/WebSocket Adapter

The custom adapter provides a generic HTTP/WebSocket endpoint (default port 8081):

```toml
[adapters.custom]
enabled = true
listen_port = 8081
```

**Sending messages via HTTP:**
```bash
curl -X POST http://localhost:8081/api/v1/message \
  -H "Content-Type: application/json" \
  -d '{"channel": "my-cli", "text": "Hello"}'
```

For a complete reference, check the default `orka.toml` file in the repository root.
