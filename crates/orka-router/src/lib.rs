use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Envelope, OutboundMessage, Payload, Result, Session};
use orka_worker::AgentHandler;
use tracing::debug;

/// A route that maps incoming messages to a specific handler based on a prefix pattern.
pub struct AgentRoute {
    /// Prefix to match against the message text (e.g., "/code", "@assistant").
    pub prefix: String,
    /// The handler to delegate to when the prefix matches.
    pub handler: Arc<dyn AgentHandler>,
    /// Whether to strip the prefix from the message before passing to the handler.
    pub strip_prefix: bool,
}

/// Routes incoming messages to different `AgentHandler` implementations based on message content.
pub struct AgentRouter {
    routes: Vec<AgentRoute>,
    default_handler: Arc<dyn AgentHandler>,
}

impl AgentRouter {
    /// Create a new router with a default (fallback) handler.
    pub fn new(default_handler: Arc<dyn AgentHandler>) -> Self {
        Self {
            routes: Vec::new(),
            default_handler,
        }
    }

    /// Add a route. Routes are evaluated in insertion order; first match wins.
    pub fn route(mut self, prefix: impl Into<String>, handler: Arc<dyn AgentHandler>, strip_prefix: bool) -> Self {
        self.routes.push(AgentRoute {
            prefix: prefix.into(),
            handler,
            strip_prefix,
        });
        self
    }
}

#[async_trait]
impl AgentHandler for AgentRouter {
    async fn handle(&self, envelope: &Envelope, session: &Session) -> Result<Vec<OutboundMessage>> {
        let text = match &envelope.payload {
            Payload::Text(t) => t,
            _ => return self.default_handler.handle(envelope, session).await,
        };

        for route in &self.routes {
            if text.starts_with(&route.prefix) {
                debug!(prefix = %route.prefix, "routing message to matched handler");

                if route.strip_prefix {
                    let stripped = text[route.prefix.len()..].trim_start().to_string();
                    let mut modified = envelope.clone();
                    modified.payload = Payload::Text(stripped);
                    return route.handler.handle(&modified, session).await;
                }

                return route.handler.handle(envelope, session).await;
            }
        }

        debug!("no route matched, using default handler");
        self.default_handler.handle(envelope, session).await
    }
}

/// A handler that delegates all messages to another handler, wrapping replies with a tag.
pub struct DelegateHandler {
    name: String,
    inner: Arc<dyn AgentHandler>,
}

impl DelegateHandler {
    pub fn new(name: impl Into<String>, inner: Arc<dyn AgentHandler>) -> Self {
        Self {
            name: name.into(),
            inner,
        }
    }
}

#[async_trait]
impl AgentHandler for DelegateHandler {
    async fn handle(&self, envelope: &Envelope, session: &Session) -> Result<Vec<OutboundMessage>> {
        let mut replies = self.inner.handle(envelope, session).await?;
        for reply in &mut replies {
            if let Payload::Text(ref mut t) = reply.payload {
                *t = format!("[{}] {}", self.name, t);
            }
        }
        Ok(replies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::{SessionId, Payload};
    use orka_worker::EchoHandler;

    #[tokio::test]
    async fn routes_by_prefix() {
        let echo = Arc::new(EchoHandler);
        let delegate = Arc::new(DelegateHandler::new("code", echo.clone()));

        let router = AgentRouter::new(echo.clone())
            .route("/code ", delegate, true);

        let session = Session::new("custom", "user1");

        // Should match /code prefix and strip it
        let env = Envelope::text("custom", SessionId::new(), "/code hello world");
        let replies = router.handle(&env, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => {
                assert!(t.contains("[code]"));
                assert!(t.contains("hello world"));
                assert!(!t.contains("/code"));
            }
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn falls_back_to_default() {
        let echo = Arc::new(EchoHandler);
        let delegate = Arc::new(DelegateHandler::new("code", echo.clone()));

        let router = AgentRouter::new(echo.clone())
            .route("/code ", delegate, true);

        let session = Session::new("custom", "user1");

        let env = Envelope::text("custom", SessionId::new(), "just a message");
        let replies = router.handle(&env, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => {
                assert!(t.contains("just a message"));
                assert!(!t.contains("[code]"));
            }
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn delegate_handler_tags_replies() {
        let echo = Arc::new(EchoHandler);
        let delegate = DelegateHandler::new("assistant", echo);

        let session = Session::new("custom", "user1");
        let env = Envelope::text("custom", SessionId::new(), "hi");
        let replies = delegate.handle(&env, &session).await.unwrap();

        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => assert!(t.starts_with("[assistant]")),
            _ => panic!("expected text"),
        }
    }
}
