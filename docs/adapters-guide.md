# Adapters Guide

Orka is designed to connect to various messaging interfaces through **Adapters**. An adapter translates incoming messages from external channels into internal `Payload::Event` envelopes and handles outgoing responses from the agent.

Currently, Orka features two built-in adapters out-of-the-box: **Telegram** and **Custom HTTP**.

## 1. Telegram Adapter

The Telegram adapter allows Orka to interact directly as a Telegram Bot. It supports both long-polling (best for local development) and webhooks (recommended for production).

### Setup Instructions

1. **Create the Bot:** Open Telegram and search for `@BotFather`. Use the `/newbot` command to create a new bot and obtain your Bot Token.
2. **Store the Secret:** Do not put the bot token directly in `orka.toml`. Instead, use Orka's encrypted secrets engine.
   ```bash
   orka secret set telegram_token "YOUR_BOT_TOKEN_HERE"
   ```
3. **Configure the Adapter:** Open `orka.toml` and configure the Telegram section:
   ```toml
   [adapters.telegram]
   bot_token_secret = "telegram_token" # The name of the secret you just set
   mode = "polling"                    # Use "webhook" in production
   parse_mode = "HTML"                 # "HTML" or "MarkdownV2"
   ```

### Webhook Configuration (Production)
For production deployments, switch `mode` to `"webhook"`. You will need to expose the Orka server publicly (e.g., via a reverse proxy like Nginx or a tunnel like Cloudflare Tunnel or Ngrok) and point Telegram to it.

```toml
[adapters.telegram]
mode = "webhook"
webhook_url = "https://your-public-domain.com/webhook/telegram"
webhook_port = 8443
```

## 2. Custom HTTP / WebSocket Adapter

The custom adapter is a generic, unauthenticated adapter built primarily for local development, custom frontends, and A2A testing.

When running Orka, it binds an HTTP and WebSocket server (by default on port `8081`).

### Sending Messages

You can send a message manually via HTTP POST:

```bash
curl -X POST http://localhost:8081/api/v1/message \
  -H "Content-Type: application/json" \
  -d '{"channel": "my-custom-cli", "text": "Hello, Orka!"}'
```

### WebSocket Streaming

For real-time interactions, you can connect to the `/api/v1/ws` endpoint. This allows Orka to stream LLM responses chunk-by-chunk directly to your client.

## Future Adapters

Support for additional platforms like Discord, Slack, and WhatsApp is planned for future releases. If you need to integrate them immediately, you can either:
1. Wrap them around the Custom HTTP adapter via an external gateway.
2. Write a custom Rust adapter crate by implementing the standard `Adapter` traits available in `orka-core`.
