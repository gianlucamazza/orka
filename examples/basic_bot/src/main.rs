//! Basic Telegram Bot Example
//!
//! This example demonstrates how to build a simple Telegram bot using Orka.
//! The bot echoes back messages and provides a foundation for more complex bots.
//!
//! ## Setup
//!
//! 1. Create a Telegram bot via @BotFather and get your bot token
//! 2. Set environment variable: `export TELEGRAM_BOT_TOKEN="your_token"`
//! 3. Run: `cargo run --bin basic_bot`
//!
//! ## Architecture
//!
//! ```text
//! Telegram API → TelegramAdapter → MessageBus → EchoHandler → Outbound
//! ```

use anyhow::Result;
use orka_adapter_telegram::TelegramAdapter;
use orka_core::testing::{InMemoryBus, InMemorySessionStore};
use orka_core::traits::{ChannelAdapter, MessageBus, SessionStore};
use orka_core::types::{Envelope, OutboundMessage, Payload, SessionId};
use std::sync::Arc;
use tracing::{info, warn};

/// Simple echo handler that processes messages and generates responses.
struct EchoHandler {
    bus: Arc<dyn MessageBus>,
}

impl EchoHandler {
    fn new(bus: Arc<dyn MessageBus>) -> Self {
        Self { bus }
    }

    /// Handle incoming envelope and produce response
    async fn handle(&self, envelope: &Envelope) -> Result<()> {
        // Extract text from payload
        let text = match &envelope.payload {
            Payload::Text(t) => t.clone(),
            Payload::Command(cmd) => format!("Received command: /{}", cmd.name),
            _ => {
                info!("Ignoring non-text payload");
                return Ok(());
            }
        };

        // Create echo response
        let response = format!("🤖 Echo: {}", text);

        let outbound = OutboundMessage {
            channel: envelope.channel.clone(),
            session_id: envelope.session_id,
            payload: Payload::Text(response),
            metadata: envelope.metadata.clone(),
        };

        // Convert to envelope for publishing
        let response_envelope = Envelope::text(
            &outbound.channel,
            outbound.session_id,
            match &outbound.payload {
                Payload::Text(t) => t,
                _ => "",
            },
        );

        self.bus.publish("outbound", &response_envelope).await?;
        info!(session_id = %envelope.session_id, "responded to message");

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("basic_bot=info,orka=warn")
        .init();

    info!("Starting Basic Telegram Bot Example");

    // Get bot token from environment
    let bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
        .expect("TELEGRAM_BOT_TOKEN environment variable must be set");

    // Create in-memory bus for message passing
    let bus = Arc::new(InMemoryBus::new());
    let bus_clone = bus.clone();

    // Create session store
    let session_store = Arc::new(InMemorySessionStore::new());

    // Create Telegram adapter
    let adapter = TelegramAdapter::new(bot_token, 8080);

    // Start adapter in background
    let adapter_handle = tokio::spawn(async move {
        adapter.start(bus_clone).await.unwrap();
    });

    // Subscribe to inbound messages
    let mut subscriber = bus.subscribe("inbound").await?;

    // Create handler
    let handler = EchoHandler::new(bus.clone());

    info!("Bot is running. Send messages to your Telegram bot!");

    // Main event loop
    loop {
        tokio::select! {
            Some(envelope) = subscriber.recv() => {
                if let Err(e) = handler.handle(&envelope).await {
                    warn!(error = %e, "failed to handle message");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal");
                break;
            }
        }
    }

    // Cleanup
    drop(adapter_handle);
    info!("Bot shutdown complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_echo_handler() {
        let bus = Arc::new(InMemoryBus::new());
        let handler = EchoHandler::new(bus.clone());

        let envelope = Envelope::text("telegram", SessionId::new(), "Hello bot");

        handler.handle(&envelope).await.unwrap();

        // Verify response was published
        // In real test, we'd subscribe and check
    }
}
