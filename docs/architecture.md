# Orka Architecture

End-to-end description of how a message flows through the Orka platform.

## High-level diagram

```
User / External System
        │
        │  HTTP POST /message  (or webhook, Telegram update, Slack event, …)
        ▼
┌─────────────────────┐
│   ChannelAdapter    │  orka-adapter-{telegram,discord,slack,whatsapp,custom}
│                     │  Converts platform-specific format → Envelope
└────────┬────────────┘
         │  MessageBus.publish("inbound", envelope)
         ▼
┌─────────────────────┐
│    Redis Streams    │  orka-bus
│   ("inbound" stream)│
└────────┬────────────┘
         │  subscribe
         ▼
┌─────────────────────┐
│      Gateway        │  orka-gateway
│                     │  1. Deduplication (Redis SET NX EX)
│                     │  2. Rate limiting (Redis INCR / in-memory fallback)
│                     │  3. Session resolution (create or load)
│                     │  4. Priority routing (DM → Urgent, group → Normal)
│                     │  5. Trace context generation (W3C traceparent)
└────────┬────────────┘
         │  PriorityQueue.push(envelope)
         ▼
┌─────────────────────┐
│   Redis Sorted Set  │  orka-queue
│  (priority queue)   │  Score = priority * 10^12 - timestamp (highest first)
└────────┬────────────┘
         │  PriorityQueue.pop() — blocking, 5 s timeout
         ▼
┌─────────────────────┐
│    WorkerPool       │  orka-worker
│  (N concurrent)     │  Retries: base * 3^n backoff, DLQ after max_retries
└────────┬────────────┘
         │  AgentHandler.handle(envelope, session)
         ▼
┌─────────────────────────────────────────────────────────────┐
│                   WorkspaceHandler                          │
│                                                             │
│  1. Load workspace config (system prompt, skills, model)    │
│  2. Inject memory context (orka-memory)                     │
│  3. Inject principles (orka-experience)                     │
│  4. Apply guardrails (orka-guardrails)                      │
│  5. Agentic loop:                                           │
│       a. LLM call (orka-llm → Anthropic / OpenAI / Ollama) │
│       b. Stream chunks to client (orka-stream)              │
│       c. Execute tool calls (orka-skills)                   │
│       d. Repeat until stop reason = end_turn                │
│  6. Post-task reflection → trajectory recording             │
│     (orka-experience)                                       │
└────────┬────────────────────────────────────────────────────┘
         │  Vec<OutboundMessage>
         ▼
┌─────────────────────┐
│    Redis Streams    │  bus publish("outbound", envelope)
│  ("outbound" stream)│
└────────┬────────────┘
         │  subscribe
         ▼
┌─────────────────────┐
│   ChannelAdapter    │  Converts OutboundMessage → platform reply
└─────────────────────┘
         │
         ▼
    User sees reply
```

## Subsystems

### Message Bus (orka-bus)

Redis Streams back the `MessageBus` trait. Adapters publish inbound envelopes
to the `"inbound"` stream and subscribe to the `"outbound"` stream to deliver
replies. Each consumer group ensures at-most-once delivery per consumer.

### Priority Queue (orka-queue)

A Redis Sorted Set stores pending envelopes. The score encodes both priority
(`Urgent > Normal > Background`) and arrival time so higher-priority messages
are always processed first, with FIFO ordering within a priority tier.
Dead-letter entries are written to a separate key (`orka:dlq`).

### Session Store (orka-session)

Sessions are stored in Redis as JSON. A session represents a single user conversation
on a specific channel. It carries a `state` scratchpad that skills can read and
write for cross-turn memory within a session.

### Memory Store (orka-memory)

Long-term key-value memory, persisted in Redis. The `WorkspaceHandler` loads
relevant memory entries into the system prompt before each LLM call so the agent
has context from previous sessions.

### Knowledge / RAG (orka-knowledge)

Documents are embedded and stored in Qdrant (vector DB). At inference time,
a semantic search retrieves the most relevant passages, which are injected into
the system prompt. Ingestion pipelines can be triggered via the scheduler or
directly via the API.

### Experience System (orka-experience)

Three-phase self-learning loop:

1. **Trajectory recording** — after each task, the full interaction (messages,
   tool calls, outcomes) is serialized and stored.
2. **Online reflection** — immediately after task completion, an LLM call
   analyzes the trajectory and produces or updates _principles_ (heuristics
   about what worked and what didn't).
3. **Offline distillation** — a background job synthesizes patterns across
   many trajectories to produce higher-quality, cross-task principles.

Principles are injected into the system prompt alongside memory.

### Scheduler (orka-scheduler)

Cron-based task scheduler backed by a Redis Sorted Set. Tasks are stored with
their next-run timestamp as the score. A polling loop pops due tasks and
publishes them to the bus as `Payload::Event` envelopes.

### Guardrails (orka-guardrails)

Pre- and post-processing pipeline for `Envelope` and `OutboundMessage`.
Guardrails can block, modify, or log messages. Privileged command approval
is implemented here: commands matching a deny-list require explicit approval
before the sandbox executes them.

### Sandbox (orka-sandbox)

Isolated execution environment for the `shell` skill. Commands run in a
restricted process with configurable allow/deny lists. Execution results and
exit codes are emitted as `PrivilegedCommandExecuted` domain events.

### Secrets (orka-secrets)

`SecretManager` implementations: environment variable backend (default),
HashiCorp Vault backend, and an in-memory backend for tests. Secrets are
wrapped in `SecretValue` which is `!Clone` and zeroizes on drop.

## Skill execution

Skills are invoked by the agentic loop when the LLM emits a tool call.
The flow:

```
LLM tool_call { name: "web_search", args: { query: "…" } }
    │
    ▼
SkillRegistry.find("web_search")
    │
    ▼
Skill::execute(SkillInput { args, context })
    │  (context carries SecretManager + EventSink)
    ▼
SkillOutput { data: json!({ "results": […] }) }
    │
    ▼
Appended to messages as ToolResult, loop continues
```

Built-in skills live in `orka-skills`. External skills can be provided as
WASM plugins compiled with `orka-plugin-sdk`.

## Observability

Every significant event emits a `DomainEvent` to the `EventSink`. The
`orka-observe` crate subscribes to events and:

- Logs them via `tracing`
- Exposes them as Server-Sent Events on the `/events` endpoint
- Records metrics (token counts, latency, cost estimates)

## Configuration

Configuration is layered (later sources override earlier ones):

1. Default values (compiled in)
2. `orka.toml` (path from `ORKA_CONFIG` env var, default `./orka.toml`)
3. `ORKA__*` environment variables (double-underscore as separator)

The schema is versioned; `orka-core::migrate` handles upgrades automatically
on startup.
