pub mod routes;
pub mod types;
pub mod ws;

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    config::CustomAdapterConfig, traits::ChannelAdapter, MessageSink, OutboundMessage, Result,
};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::routes::app_router;
use crate::ws::WsRegistry;

/// Custom HTTP/WS adapter for receiving messages via REST API.
pub struct CustomAdapter {
    config: CustomAdapterConfig,
    auth_layer: Option<orka_auth::AuthLayer>,
    sink: Arc<Mutex<Option<MessageSink>>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    ws_registry: WsRegistry,
}

impl CustomAdapter {
    pub fn new(config: CustomAdapterConfig, auth_layer: Option<orka_auth::AuthLayer>) -> Self {
        Self {
            config,
            auth_layer,
            sink: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            ws_registry: WsRegistry::new(),
        }
    }

    pub fn ws_registry(&self) -> &WsRegistry {
        &self.ws_registry
    }
}

#[async_trait]
impl ChannelAdapter for CustomAdapter {
    fn channel_id(&self) -> &str {
        "custom"
    }

    async fn start(&self, sink: MessageSink) -> Result<()> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let router = app_router(sink.clone(), self.ws_registry.clone(), self.auth_layer.clone());

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
        let count = self.ws_registry.send_to_session(&msg.session_id, &text).await;
        if count == 0 {
            warn!(session_id = %msg.session_id, "no active WS connections, message dropped");
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
}
