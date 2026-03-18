# orka-worker

Concurrent worker pool that pops messages from the priority queue and dispatches
them to agent handlers.

## Components

| Type               | Description                                                                                                  |
| ------------------ | ------------------------------------------------------------------------------------------------------------ |
| `WorkerPool`       | Spawns N concurrent workers; each pops from the queue, invokes the handler, and publishes replies to the bus |
| `AgentHandler`     | Trait implemented by message handlers (`handle(&Envelope, &Session) -> Vec<OutboundMessage>`)                |
| `WorkspaceHandler` | Production handler — runs an LLM agentic loop with skill execution, tool calls, and streaming                |
| `EchoHandler`      | Trivial handler that echoes the message back (useful for testing)                                            |
| `CommandRegistry`  | Maps slash-command names to handler functions                                                                |

## Retry policy

Failed messages are re-enqueued with exponential backoff (`base * 3^retry`).
After `max_retries` attempts the envelope is sent to the dead-letter queue.

## Usage

```rust
let pool = WorkerPool::new(
    queue, sessions, bus, handler, event_sink,
    /* concurrency */ 4,
    /* max_retries */ 3,
);
pool.run(shutdown_token).await?;
```
