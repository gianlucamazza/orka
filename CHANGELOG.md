# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.6.0] - 2026-04-05

### Added

- add 8 missing mobile API endpoints

### Fixed

- auto-migrate plaintext secrets when encryption key is added

### Changed

- consolidate migrate_plaintext_secrets on SecretManager trait
## [1.5.0] - 2026-04-05

### Added

- add ScheduleTriggered event and observability coverage
- emit LlmRequest domain event before streaming completion
- add GenerationStarted chunk for typing indicators
- add cursor pagination and read receipts for conversations
- cursor pagination, typing indicators, and read receipts
- close API gaps identified in client inventory

### Changed

- fix clippy warnings from feature additions
## [1.4.0] - 2026-04-04

### Added

- add archive and delete conversation endpoints
- harden auth and server resiliency
- add fan-out limits and safer remote sessions
- differentiate mobile API rate limits for reads and writes

### Fixed

- replace assigning_clones with clone_from in cli commands
- use clone_from in bootstrap to_runtime converters
- replace unwrap_err with is_err_and and elevate test structs
- use explicit match arm for VectorStoreBackend::Qdrant

### Changed

- narrow config boundaries and decouple agent graph builder
- simplify workspace error types
- simplify archived conversation filtering

### Documentation

- add live demo pipeline and refreshed assets
- include latest fixes in v1.4.0 notes
- update CHANGELOG for 1.4.0 with audit completion entries
## [1.3.0] - 2026-04-03

### Added

- forward ThinkingDelta, ToolExec, and AgentSwitch stream events to SSE
- add artifact support

### Fixed

- resolve pre-existing warnings in orka-server
- resolve expect_used, too-many-lines, and wildcard-match warnings in orka-server
- bump cargo-chef to 0.1.77 to support edition 2024
- add libfontconfig-dev and libfreetype6-dev to build stage
- drop empty-name tool calls and orphaned results in OpenAI path
- fix all clippy warnings and complete artifact/RichInput wiring

### Changed

- extract stream_chunk_to_sse helper to remove too_many_lines allow
## [1.2.0] - 2026-04-01

### Changed

- typed error variants, Arc<dyn Skill>, onboard extraction (v1.2.0)
## [1.1.0] - 2026-04-01

### Added

- add conversation store, JWT auth, and mobile API
- wire conversation store, JWT auth, and mobile hub into Bootstrap
- add orka-conversation crate and mobile router
- add pagination, OpenAPI registration, and auth tests
- add QR pairing and device session auth

### Fixed

- eliminate unwrap() in budget duration check

### Changed

- normalize workspace deps and remove unused insta dev-dep
- replace std::fs with tokio::fs in worktree operations
- serialize task before store to eliminate clone
- offload blocking write to spawn_blocking in AuditSink
- replace ArcSwap with RwLock in SwappableLlmClient
- split Gateway::new() into GatewayDeps + GatewayConfig
- extract Bootstrap struct from run() function
- extract AgentNodeRunner and WorkspaceHandlerDeps
- pass CompletionOptions by reference in LlmClient trait
- unify image dep at 0.25 and normalize adapter-custom dev-deps
- reduce too_many_arguments with config structs (N1)
- consolidate 7 small crates into 4 existing crates
- feature-gate optional subsystem dependencies

### Documentation

- update for crate consolidation

### Build

- make local embeddings opt-in by default

### Sdk

- tolerate clippy lint drift in hello plugin
- format hello plugin lint allow
## [1.0.0] - 2026-03-29

### Added

- add release tooling config (git-cliff, cargo-release, commitlint)
- add lock-free LLM client hot-swap and .env watcher
- add CLI enhancements, OS approval, workspace registry, and streaming support
- dual license MIT/Apache-2.0, upgrade deps, harden install script
- add retry module, stream improvements, config extensions, and ergonomic constructors
- add constructor methods, complete_with_options, and non_exhaustive annotations
- handle new domain event kinds and add metrics
- add self-learning experience system with trajectory collection, reflection, and distillation
- integrate experience system into workspace handler
- wire experience service and distillation loop
- handle TrajectoryRecorded and DistillationCompleted events
- change WASM plugin input to structured JSON args
- rewrite adapter with webhook, media and auth guard
- add package_updates skill
- add build-time version info, sd_notify and /version endpoint
- add systemd fs drop-in for home directory access
- add Interact permission level and reassign skill tiers
- extract shared WASM engine to orka-wasm crate
- introduce multi-agent graph executor
- config v3 with multi-agent definitions
- upgrade Discord, Slack, Telegram, WhatsApp
- wire graph execution and misc improvements
- CLI enhancements, ClaudeCode skill, stream/context improvements, and polish
- 8 new CLI commands, 14 REST endpoints, session listing, and graph accessors
- add demo GIF to README and refine CLI internals
- WS channel filter and source_channel metadata propagation
- add MCP bridge (claude-channel) for Claude Code ↔ Orka integration
- soft skills, MCP HTTP/OAuth, WASM Component Model, eval framework, TUI dashboard
- overhaul Telegram command system
- parse compound command strings with POSIX shell quoting
- add fs_edit, JS sandbox, stdin/env passthrough, code guardrails
- stream send output and raise default timeout to 120s
- tool-level guardrails, keyword soft skill selection, MCP version alignment
- inject shell context into AI prompts and improve UX
- Unify `BuildContext` in `pipeline` and introduce `ContextCoordinator` with new providers for prompt context management.
- complete architecture with Rust 2026 best practices
- complete remaining architecture tasks with Rust 2026 best practices
- add LLM moderation and prompt injection detection
- initial scaffold for orka-eval and codebase sync
- align OS integration and security with excellence audit
- replace claude_code with coding_delegate, remove approval system
- multi-agent graph execution and dynamic prompt sections
- migrate from rustyline to reedline for interactive input
- add Debian and Fedora packaging with CI
- add config v6 with node kinds, history filter, typed enums, and plugin capabilities
- add adaptive thinking support and stream consumer
- wire graph node kinds, guardrails, and history filtering into executor
- add orka doctor diagnostic command
- add git and protocol configuration foundation
- add checkpoint persistence and git skills crates
- add planner/reducer flow and checkpoint-aware execution
- implement A2A v1.0 routes, discovery, streaming, and CLI support
- add skill timeout and concurrency config to AgentConfig
- add OpenCode coding backend support
- refactor coding_delegate to module with streaming and cancellation
- add autonomous research campaign subsystem
- add create_chart skill with inline PNG delivery
- add onboarding wizard, inline media rendering, and file-backed secrets
- implement cross-session history persistence via MemoryStore
- add configurable TTL to RedisResearchStore with builder API
- add AgentStopReason and propagate through execution pipeline
- extract ChatRenderer/TermCaps and surface stop-reason warnings
- normalize max_iterations → max_turns without version bump
- gate optional backends behind cargo features
- add typed memory layers, list/delete ops, and progress bridge
- introduce FactStore and semantic memory skills
- inject semantic facts into agent system prompt
- add list_principles and forget_principle operations
- replace /reset with /memory command and wire FactStore
- wire FactStore through bootstrap and agent executor
- add auth_kind config for flexible API key vs bearer token auth
- support auth_kind for reflection LLM credential resolution
- support Auto auth_kind with inferred token type and add env_watcher tests
- switch native-tls to rustls and gate local-embeddings for ARM
- add aarch64 cross-compilation support
- add ARM release builds, Docker multi-arch, and homelab push
- add moonshot provider support
- add SecretStr type for zeroize-on-drop string credentials
- add webhook secret verification, hide token from URLs, and migrate to SecretStr
- add HMAC-SHA256 webhook verification and migrate to SecretStr
- add HMAC-SHA256 webhook verification and migrate to SecretStr
- wire structured errors, deterministic routing, harden prefix matching, and migrate to SecretStr
- add lazy Qdrant reconnection, embedding retry, and migrate to SecretStr
- wire SecretStr credentials through providers, adapters, and env watcher
- wrap API keys in SecretStr at init command entry point
- add distributed execution lock and fix TOCTOU race
- extend validation to all subsections
- add debug tracing
- add WASM target build job
- add PluginInput.get_str helper to align raw and Component Model SDKs
- add architecture doctor checks

### Fixed

- resolve all clippy warnings across workspace
- implement Qdrant filter support in search()
- add filter param to VectorStore::list_documents()
- make MockVectorStore respect filters in search() and list_documents()
- fix prompt width, builtin parsing and streaming UX
- remove spurious streamed_this_turn reset on Done
- harden systemd unit with least-privilege fs restrictions
- handle empty arrays in PKGBUILD and install.sh config patching
- rename install() to do_install() and fix SUDOERS_TMP trap
- complete WebSocket close handshake on exit
- move send status output to stderr; fix status ready/checks parsing
- enforce workspace:cwd for tilde paths and strengthen system prompt
- dispatch slash commands directly in WorkerPoolGraph
- restore conversation context after tool-call turns
- improve WebSocket close handshake with SeqCst ordering and flush
- resolve race condition in AsyncServiceContainer
- update tests for McpTransportConfig API drift
- fall back to /etc/orka/orka.toml for native installs
- apply clippy lints for inline format args and method refs
- resolve WebSocket disconnects on long agent iterations
- apply idle timeout to `orka send` command
- move audit log to /var/lib/orka and document path constraints
- add plotters ttf feature for headless text rendering
- resolve clippy warnings across adapters, eval, experience, gateway, and sandbox
- use write! macro instead of format!+push_str in reflector
- resolve crash, security, and data correctness bugs
- persist history on HITL interrupt and deduplicate trigger message
- wrap all write/delete pairs in atomic pipelines
- implement correct SCAN cursor iteration in list()
- correct StackedBar, Combo, Histogram, and y-axis semantics
- remove dead code and silent failure modes across crates
- data integrity and guardrail correctness (Tier 1 round 2)
- validate JSON-RPC version, cap in-memory store, MGET batch-fetch, extract push helper
- make from_checkpoint sync, harden history deserialisation, code cleanup
- router correctness and clippy fixes
- clippy and code-quality fixes across the CLI
- clippy and idiomatic improvements across commands
- abort on config validation or migration failure
- add missing bus field in node_runner test fixture
- support OAuth tokens in Anthropic client
- add wildcard arm for non-exhaustive LlmAuthKind match
- add auth_kind/auth_token fields to migrate schema allowlist
- add jitter to backoff_delay and improve Telegram error logging
- correct default Qdrant URL to gRPC port 6334
- add LlmProviderConfig::for_provider constructor to fix non-exhaustive errors
- fix pre-existing compilation and logic errors in integration tests
- suppress warnings and apply lint fixes across workspace
- fix unnecessary mut and borrowed value in orka-cli
- allow type_complexity in interactive_phase1
- fix Cross.toml for aarch64 — use libfreetype6-dev and install clang
- support OpenAI o-series and gpt-5 model families
- make agent temperature optional for model compatibility
- always use max_completion_tokens, skip reasoning for non-o-series
- temperature inheritance + streaming token tracking
- temperature inheritance + streaming token tracking (#4)
- add interaction ACK, fix heartbeat sequence, and migrate to SecretStr
- isolate reflection failures and prevent concurrent distillation
- fix production healthcheck command
- resolve cargo-deny license and advisory issues
- fix remaining pedantic lint errors
- pass SecretStr by reference in TelegramAdapter::new
- rewrite if-let-else as let-else in experience service
- allow expect_used in test modules

### Changed

- extract resolve_api_key helper to DRY up LLM provider init
- migrate skills and adapters to new core constructors
- update bus, gateway, memory, and infra crates
- replace inline retry loops with retry_with_backoff
- remove test-util feature gate
- expand config with doc comments, backend overrides and Telegram fields
- remove dead NATS stub
- unify ChatMessage with typed Role enum
- modernize scripts/install.sh with umask, install(), flags, and robustness fixes
- split config.rs into modular config directory
- config schema v3-v5 migrations and API cleanup
- align all adapters with updated config API
- align bootstrap and commands with multi-agent config
- extract SessionLock and DeadLetterQueue traits (ISP)
- extract Dispatcher trait and split workspace_handler into module
- adopt Dispatcher, DeadLetterQueue, SessionLock, and typed config enums
- eliminate magic numbers and adopt typed enums across crates
- rename max_iterations to max_turns
- align DEFAULT_MAX_ITERATIONS and SOUL.md to max_turns rename
- type config enums and harden workspace lookup
- introduce crate-specific error types
- harden typed ids and sanitize graph history
- make check ids opaque
- replace /reset with /memory in completions and chat UI
- extract composed config into orka-config
- move domain config to owning crates, remove container

### Documentation

- add rustdoc to orka-core public API and enforce missing_docs lint
- enhance CONTRIBUTING.md with commit conventions and architecture guide
- fix MSRV inconsistency (1.75 → 1.85)
- add crate READMEs and architecture overview
- add doc comments across all crates
- expand config reference table and orka.toml inline comments
- add deployment, skill-development, mcp-guide, experience-system, eval-guide, expand SECURITY.md
- replace ASCII diagrams with Mermaid and add demo tape files
- update README description and project structure
- validate and fix architecture diagrams
- replace \n with <br/> in Mermaid diagrams for correct rendering
- implement documentation audit improvements
- cleanup and consolidate documentation
- reorganize documentation into guides/reference/internal
- translate to English, update architecture, remove obsolete plans
- add internal architecture, security, and coverage reports
- update README and CONTRIBUTING for current state
- update README with current endpoints and port layout
- refresh reference docs and navigation index
- update analysis and security reports
- update TOOLS.md for current skill set and refresh Cargo.lock
- document auth_kind config and ANTHROPIC_AUTH_TOKEN env var
- align docs with config refactoring and container removal
- add architecture principles reference and update CLI reference

### Demo

- record all GIF demos and fix send.tape quote escaping

### Security

- remove hardcoded user home paths from tests and config
- move homelab registry URL out of Justfile into env var
[1.6.0]: https://github.com/gianlucamazza/orka/compare/v1.5.0...v1.6.0
[1.5.0]: https://github.com/gianlucamazza/orka/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/gianlucamazza/orka/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/gianlucamazza/orka/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/gianlucamazza/orka/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/gianlucamazza/orka/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/gianlucamazza/orka/compare/v0.1.0...v1.0.0

