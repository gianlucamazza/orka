//! Default value functions for configuration.
//!
//! This module centralizes all default value functions used across the
//! configuration system, ensuring consistency and discoverability.

/// System-wide configuration file path (native package installs).
pub const SYSTEM_CONFIG_PATH: &str = "/etc/orka/orka.toml";

/// Default host for the HTTP server.
pub fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// Default port for the HTTP server.
pub const fn default_port() -> u16 {
    8080
}

/// Default workspace directory path.
pub fn default_workspace_dir() -> String {
    "./workspace".to_string()
}

/// Default bus backend type.
pub fn default_bus_backend() -> String {
    "redis".to_string()
}

/// Default block timeout for message bus in milliseconds.
pub const fn default_bus_block_ms() -> u64 {
    5000
}

/// Default batch size for message bus operations.
pub const fn default_bus_batch_size() -> usize {
    100
}

/// Default initial backoff for bus connection errors in seconds.
pub const fn default_bus_backoff_initial_secs() -> u64 {
    1
}

/// Default maximum backoff for bus connection errors in seconds.
pub const fn default_bus_backoff_max_secs() -> u64 {
    60
}

/// Default Redis URL.
pub fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

/// Default Qdrant vector store URL.
pub fn default_qdrant_url() -> String {
    "http://localhost:6333".to_string()
}

/// Default log level.
pub fn default_log_level() -> String {
    "info".to_string()
}

/// Default worker concurrency.
pub const fn default_concurrency() -> usize {
    4
}

/// Default retry base delay in milliseconds.
pub const fn default_retry_base_delay_ms() -> u64 {
    1000
}

/// Default maximum entries for memory store.
pub const fn default_max_entries() -> usize {
    1000
}

/// Default memory backend.
pub fn default_backend_auto() -> String {
    "auto".to_string()
}

/// Default API key header name.
pub fn default_api_key_header() -> String {
    "X-API-Key".to_string()
}

/// Default sandbox backend.
pub fn default_sandbox_backend() -> String {
    "process".to_string()
}

/// Default sandbox timeout in seconds.
pub const fn default_timeout_secs() -> u64 {
    30
}

/// Default maximum memory for sandbox in bytes (64MB).
pub const fn default_max_memory_bytes() -> usize {
    64 * 1024 * 1024
}

/// Default maximum output bytes for sandbox.
pub const fn default_max_output_bytes() -> usize {
    1024 * 1024
}

/// Default soft skill selection mode.
pub fn default_soft_skill_selection_mode() -> String {
    "auto".to_string()
}

/// Default session TTL in seconds.
pub const fn default_session_ttl_secs() -> u64 {
    86400
}

/// Default maximum retries for queue operations.
pub const fn default_max_retries() -> u32 {
    3
}

/// Default audit output destination.
pub fn default_audit_output() -> String {
    "stdout".to_string()
}

/// Default batch size for observability.
pub const fn default_observe_batch_size() -> usize {
    100
}

/// Default flush interval for observability in milliseconds.
pub const fn default_observe_flush_interval_ms() -> u64 {
    1000
}

/// Default observability backend.
pub fn default_observe_backend() -> String {
    "stdout".to_string()
}

/// Default gateway rate limit (0 = unlimited).
pub const fn default_gateway_rate_limit() -> u32 {
    0
}

/// Default deduplication TTL for gateway in seconds.
pub const fn default_gateway_dedup_ttl_secs() -> u64 {
    300
}

/// Default LLM temperature.
pub const fn default_temperature() -> f32 {
    0.7
}

/// Default LLM max tokens.
pub const fn default_max_tokens() -> u32 {
    4096
}

/// Default LLM `top_p`.
pub const fn default_top_p() -> f32 {
    1.0
}

/// Default agent ID.
pub fn default_agent_id() -> String {
    "orka".to_string()
}

/// Default agent name.
pub fn default_agent_name() -> String {
    "Orka".to_string()
}

/// Default agent model.
pub fn default_model() -> String {
    "claude-sonnet-4-6".to_string()
}

/// Default max iterations for agent.
pub const fn default_max_iterations() -> usize {
    10
}

/// Default tool result max chars.
pub const fn default_tool_result_max_chars() -> usize {
    8000
}

/// Default OS enabled state.
pub const fn default_os_enabled() -> bool {
    false
}

/// Default OS permission level.
pub fn default_os_permission_level() -> String {
    "read-only".to_string()
}

/// Default Claude Code enabled state.
pub const fn default_claude_code_enabled() -> bool {
    false
}

/// Default Codex enabled state.
pub const fn default_codex_enabled() -> bool {
    false
}

/// Default coding delegate context injection.
pub const fn default_coding_inject_workspace_context() -> bool {
    true
}

/// Default coding delegate verification requirement.
pub const fn default_coding_require_verification() -> bool {
    false
}

/// Default coding delegate working directory override policy.
pub const fn default_coding_allow_working_dir_override() -> bool {
    true
}

/// Default coding delegate timeout in seconds.
pub const fn default_coding_timeout_secs() -> u64 {
    300
}

/// Default per-skill execution timeout in seconds.
pub const fn default_skill_timeout_secs() -> u64 {
    120
}

/// Default sudo allowed state.
pub const fn default_sudo_allowed() -> bool {
    false
}

/// Default web search provider.
pub fn default_web_search_provider() -> String {
    "none".to_string()
}

/// Default max web search results.
pub const fn default_web_max_results() -> usize {
    5
}

/// Default max characters to read from web page.
pub const fn default_web_max_read_chars() -> usize {
    20_000
}

/// Default max content characters for web results.
pub const fn default_web_max_content_chars() -> usize {
    8_000
}

/// Default web cache TTL in seconds.
pub const fn default_web_cache_ttl_secs() -> u64 {
    3600
}

/// Default web read timeout in seconds.
pub const fn default_web_read_timeout_secs() -> u64 {
    15
}

/// Default webhook port for custom adapter.
pub const fn default_webhook_port() -> u16 {
    8081
}

/// Default scheduler enabled state.
pub const fn default_scheduler_enabled() -> bool {
    false
}

/// Default scheduler poll interval in seconds.
pub const fn default_scheduler_poll_interval_secs() -> u64 {
    30
}

/// Default maximum number of concurrent scheduler tasks.
pub const fn default_scheduler_max_concurrent() -> usize {
    4
}

/// Default LLM request timeout in seconds.
pub const fn default_llm_timeout_secs() -> u64 {
    30
}

/// Default LLM maximum tokens per response.
pub const fn default_llm_max_tokens() -> u32 {
    8192
}

/// Default LLM maximum retry attempts on transient failures.
pub const fn default_llm_max_retries() -> u32 {
    2
}

/// Default vector store collection name.
pub fn default_collection_name() -> String {
    "orka_knowledge".to_string()
}

/// Default vector dimension.
pub const fn default_vector_dimension() -> usize {
    768
}

/// Default chunk size for text splitting.
pub const fn default_chunk_size() -> usize {
    512
}

/// Default chunk overlap.
pub const fn default_chunk_overlap() -> usize {
    50
}

/// Default knowledge `top_k`.
pub const fn default_top_k() -> usize {
    5
}

/// Default knowledge score threshold.
pub const fn default_score_threshold() -> f32 {
    0.7
}

/// Default experience enabled state.
pub const fn default_experience_enabled() -> bool {
    false
}

/// Default experience reflection trigger.
pub const fn default_reflection_trigger() -> usize {
    100
}

/// Default experience storage backend.
pub fn default_experience_backend() -> String {
    "memory".to_string()
}

/// Default experience distillation interval in seconds.
pub const fn default_experience_distillation_interval_secs() -> u64 {
    3600
}

/// Default MCP server transport.
pub fn default_mcp_transport() -> String {
    "stdio".to_string()
}

/// Default Slack port.
pub const fn default_slack_port() -> u16 {
    3000
}

/// Default `WhatsApp` port.
pub const fn default_whatsapp_port() -> u16 {
    3000
}

/// Default custom adapter host.
pub fn default_custom_host() -> String {
    "0.0.0.0".to_string()
}

/// Default custom adapter port.
pub const fn default_custom_port() -> u16 {
    8080
}

/// Default prompt template directory.
pub fn default_prompt_template_dir() -> String {
    "./prompts".to_string()
}

/// Default prompt template extension.
pub fn default_prompt_template_ext() -> String {
    "hbs".to_string()
}

/// Default prompt cache size.
pub const fn default_prompt_cache_size() -> usize {
    100
}

/// Default graph execution mode.
pub fn default_graph_execution_mode() -> String {
    "sequential".to_string()
}

/// Default graph max hops.
pub const fn default_max_hops() -> usize {
    10
}

/// Default config version.
pub const fn default_config_version() -> u32 {
    6
}

/// Default guardrails enabled state.
pub const fn default_guardrails_enabled() -> bool {
    false
}

/// Default A2A discovery enabled state.
pub const fn default_a2a_discovery_enabled() -> bool {
    false
}

/// Default A2A store backend: in-memory (no persistence).
pub fn default_a2a_store_backend() -> String {
    "memory".to_string()
}

/// Default web user agent.
pub fn default_web_user_agent() -> String {
    format!("Orka/{} (Web Agent)", env!("CARGO_PKG_VERSION"))
}

/// Default empty string (for serde defaults).
pub fn empty_string() -> String {
    String::new()
}

/// Default empty vec (for serde defaults).
pub fn empty_vec<T>() -> Vec<T> {
    Vec::new()
}
