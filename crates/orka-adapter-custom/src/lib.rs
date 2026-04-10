//! Custom HTTP/WebSocket adapter for direct API integration.

#![warn(missing_docs)]

/// Custom adapter configuration.
pub mod config;
/// Axum route handlers for the custom HTTP/WebSocket adapter.
pub mod routes;
/// Request/response types for the custom adapter API.
pub mod types;
/// WebSocket connection registry for per-session fan-out.
pub mod ws;

use std::sync::Arc;

use async_trait::async_trait;
pub use config::CustomAdapterConfig;
use orka_core::{InteractionSink, OutboundMessage, Result, StreamRegistry, traits::ChannelAdapter};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::{routes::app_router, ws::WsRegistry};

/// Custom HTTP/WS adapter for receiving messages via REST API.
pub struct CustomAdapter {
    config: CustomAdapterConfig,
    auth_layer: Option<orka_auth::AuthLayer>,
    sink: Arc<Mutex<Option<InteractionSink>>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    ws_registry: WsRegistry,
    stream_registry: StreamRegistry,
}

impl CustomAdapter {
    /// Create a new adapter with an optional stream registry for real-time
    /// streaming.
    pub fn new(
        config: CustomAdapterConfig,
        auth_layer: Option<orka_auth::AuthLayer>,
        stream_registry: StreamRegistry,
    ) -> Self {
        Self {
            config,
            auth_layer,
            sink: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            ws_registry: WsRegistry::new(),
            stream_registry,
        }
    }

    /// Access the WS registry for outbound final messages.
    pub fn ws_registry(&self) -> &WsRegistry {
        &self.ws_registry
    }
}

#[async_trait]
impl ChannelAdapter for CustomAdapter {
    fn channel_id(&self) -> &'static str {
        "custom"
    }

    async fn start(&self, sink: InteractionSink) -> Result<()> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let router = app_router(
            sink.clone(),
            self.ws_registry.clone(),
            self.stream_registry.clone(),
            self.auth_layer.clone(),
            self.trust_level(),
            self.config.workspace.clone(),
        );

        *self.sink.lock().await = Some(sink);

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(orka_core::Error::Io)?;

        let actual_addr = listener.local_addr().map_err(orka_core::Error::Io)?;
        info!("Custom adapter listening on {actual_addr}");

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().await = Some(tx);

        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .ok();
        });

        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let text = serde_json::to_string(&msg).map_err(orka_core::Error::Serialization)?;
        let count = self
            .ws_registry
            .send_to_session(&msg.session_id, &text)
            .await;
        if count == 0 {
            debug!(session_id = %msg.session_id, "no active WS connections, message dropped");
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
            info!("Custom adapter shutdown signal sent");
        } else {
            warn!("Custom adapter shutdown called but no server was running");
        }
        Ok(())
    }

    fn capabilities(&self) -> orka_core::CapabilitySet {
        use orka_core::Capability;
        [
            Capability::TextInbound,
            Capability::TextOutbound,
            Capability::StreamingDeltas,
            Capability::MediaInbound,
            Capability::MediaOutbound,
            Capability::ConversationControl,
            Capability::FileUpload,
            Capability::WebsocketBidirectional,
        ]
        .into_iter()
        .collect()
    }

    fn integration_class(&self) -> orka_core::IntegrationClass {
        orka_core::IntegrationClass::ProductClient
    }

    fn trust_level(&self) -> orka_core::TrustLevel {
        orka_core::TrustLevel::UserAuthenticated
    }
}
