# orka-llm

LLM client abstractions and provider implementations for Orka.

## Providers

| Type                 | Description                                                                                        |
| -------------------- | -------------------------------------------------------------------------------------------------- |
| `AnthropicClient`    | Anthropic Claude API (text, streaming, tool use, extended thinking)                                |
| `OpenAiClient`       | OpenAI-compatible API (text, streaming, tool use)                                                  |
| `OllamaClient`       | Ollama inference — local (`http://localhost:11434/v1`) or Ollama Cloud (`https://ollama.com/v1`)   |
| `LlmRouter`          | Routes requests by model-name prefix to the right provider; includes per-provider circuit breakers |
| `SwappableLlmClient` | Lock-free hot-swappable wrapper — swap the active client at runtime without downtime               |

## Core trait

```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, messages: &[ChatMessage], opts: &CompletionOptions)
        -> Result<CompletionResponse>;
    async fn stream(&self, messages: &[ChatMessage], opts: &CompletionOptions)
        -> Result<LlmStream>;
}
```

## Routing example

```rust
let router = LlmRouter::new()
    .add("claude-", anthropic_client)
    .add("gpt-",    openai_client)
    .add("llama",   ollama_client)   // works for both local and cloud

// Picks AnthropicClient based on the "claude-" prefix
let resp = router.complete(&messages, &opts).await?;
```

## Context management

`orka_llm::context` provides token estimation and history-truncation utilities
to keep conversations within a model's context window.
