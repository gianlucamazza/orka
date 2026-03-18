# orka-gateway

Inbound message gateway that bridges channel adapters to the worker queue.

## Responsibilities

1. **Deduplication** — uses Redis `SET NX EX` to reject duplicate message IDs within a configurable TTL
2. **Rate limiting** — sliding-window counter per session (Redis-backed with in-memory fallback)
3. **Session resolution** — creates sessions on first contact, looks them up on subsequent messages
4. **Priority routing** — promotes direct messages to `Urgent`, group messages stay `Normal`
5. **Trace context** — generates W3C traceparent headers if the adapter didn't supply them
6. **Enqueueing** — pushes the envelope onto the priority queue for workers to pick up

## Flow

```
ChannelAdapter
     │  publishes to "inbound" stream
     ▼
  Gateway.run()
     │  dedup → rate-limit → session → enqueue → ack
     ▼
PriorityQueue
     │
     ▼
WorkerPool
```

## Usage

```rust
let gateway = Gateway::new(
    bus, sessions, queue, workspace_loader, event_sink,
    Some(&redis_url),
    /* rate_limit (msgs/min) */ 60,
    /* dedup_ttl_secs */ 300,
);
gateway.run(shutdown_token).await?;
```
