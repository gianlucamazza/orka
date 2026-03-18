# orka-core

Foundational types, traits, and error handling for the Orka agent orchestration framework.
Every other crate in the workspace depends on this one.

## What's in here

| Module    | Contents                                                                                                                                              |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `traits`  | Core abstractions: `ChannelAdapter`, `MessageBus`, `SessionStore`, `MemoryStore`, `PriorityQueue`, `EventSink`, `Skill`, `SecretManager`, `Guardrail` |
| `types`   | `Envelope`, `OutboundMessage`, `Session`, `Payload`, `DomainEvent`, `SkillInput/Output`, IDs                                                          |
| `error`   | `Error` enum covering all subsystems; `Result<T>` alias                                                                                               |
| `config`  | `OrkaConfig` and sub-configs loaded from TOML + environment                                                                                           |
| `testing` | In-memory test doubles for all core traits                                                                                                            |
| `retry`   | Generic `retry_with_backoff` executor                                                                                                                 |
| `stream`  | `StreamRegistry` for real-time LLM response streaming                                                                                                 |
| `migrate` | Config schema versioning and migration engine                                                                                                         |

## Key types

```rust
// Every inbound message is wrapped in an Envelope
let env = Envelope::text("telegram", session_id, "Hello!");

// Skills receive SkillInput and return SkillOutput
let input = SkillInput::new(HashMap::from([
    ("query".into(), json!("weather in Rome")),
]));

// Domain events flow to observers
let event = DomainEvent::new(DomainEventKind::HandlerCompleted { ... });
```

## In tests

```rust
use orka_core::testing::{InMemoryBus, InMemorySessionStore, NoopEventSink};

let bus = Arc::new(InMemoryBus::new());
let sessions = Arc::new(InMemorySessionStore::default());
```
