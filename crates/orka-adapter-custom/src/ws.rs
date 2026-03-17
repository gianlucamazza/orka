use std::collections::HashMap;
use std::sync::Arc;

use orka_core::SessionId;
use tokio::sync::{Mutex, mpsc};

#[derive(Clone, Default)]
pub struct WsRegistry {
    inner: Arc<Mutex<HashMap<SessionId, Vec<mpsc::UnboundedSender<String>>>>>,
}

impl WsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new WebSocket connection for the given session.
    /// Returns the (sender, receiver) pair — the receiver yields messages to forward to the WS client.
    pub async fn register(
        &self,
        session_id: SessionId,
    ) -> (
        mpsc::UnboundedSender<String>,
        mpsc::UnboundedReceiver<String>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut map = self.inner.lock().await;
        map.entry(session_id).or_default().push(tx.clone());
        (tx, rx)
    }

    /// Remove a specific sender from the registry for the given session.
    pub async fn deregister(&self, session_id: &SessionId, sender: &mpsc::UnboundedSender<String>) {
        let mut map = self.inner.lock().await;
        if let Some(senders) = map.get_mut(session_id) {
            senders.retain(|s| !s.same_channel(sender));
            if senders.is_empty() {
                map.remove(session_id);
            }
        }
    }

    /// Broadcast a text message to all active WS connections for the given session.
    /// Prunes closed senders. Returns the number of successful deliveries.
    pub async fn send_to_session(&self, session_id: &SessionId, text: &str) -> usize {
        let mut map = self.inner.lock().await;
        let senders = match map.get_mut(session_id) {
            Some(s) => s,
            None => return 0,
        };

        let mut delivered = 0;
        senders.retain(|tx| {
            match tx.send(text.to_string()) {
                Ok(()) => {
                    delivered += 1;
                    true // keep
                }
                Err(_) => false, // closed, prune
            }
        });

        if senders.is_empty() {
            map.remove(session_id);
        }

        delivered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_and_send() {
        let registry = WsRegistry::new();
        let session = SessionId::new();
        let (_tx, mut rx) = registry.register(session).await;

        let count = registry.send_to_session(&session, "hello").await;
        assert_eq!(count, 1);

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn deregister_cleanup() {
        let registry = WsRegistry::new();
        let session = SessionId::new();
        let (tx, _rx) = registry.register(session).await;

        registry.deregister(&session, &tx).await;

        let count = registry.send_to_session(&session, "gone").await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn multiple_connections() {
        let registry = WsRegistry::new();
        let session = SessionId::new();
        let (_tx1, mut rx1) = registry.register(session).await;
        let (_tx2, mut rx2) = registry.register(session).await;

        let count = registry.send_to_session(&session, "broadcast").await;
        assert_eq!(count, 2);

        assert_eq!(rx1.recv().await.unwrap(), "broadcast");
        assert_eq!(rx2.recv().await.unwrap(), "broadcast");
    }

    #[tokio::test]
    async fn closed_sender_pruning() {
        let registry = WsRegistry::new();
        let session = SessionId::new();

        let (_tx1, mut rx1) = registry.register(session).await;
        let (_tx2, rx2) = registry.register(session).await;

        // Drop rx2 to close the channel
        drop(rx2);

        let count = registry.send_to_session(&session, "after drop").await;
        assert_eq!(count, 1);

        assert_eq!(rx1.recv().await.unwrap(), "after drop");
    }
}
