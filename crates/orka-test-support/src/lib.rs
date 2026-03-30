//! Shared integration test support helpers.

use std::env;

use anyhow::Result;
use testcontainers::{ContainerAsync, GenericImage, core::WaitFor, runners::AsyncRunner};
use testcontainers_modules::redis::Redis;
use uuid::Uuid;

/// Keeps either a CI-provided Redis URL or a locally spawned Redis container
/// alive.
pub struct RedisService {
    url: String,
    _container: Option<ContainerAsync<Redis>>,
}

impl RedisService {
    /// Resolve Redis from `REDIS_URL` or start a local ephemeral container.
    pub async fn discover() -> Result<Self> {
        if let Some(url) = env_url("REDIS_URL") {
            return Ok(Self {
                url,
                _container: None,
            });
        }

        let container = Redis::default().start().await?;
        let port = container.get_host_port_ipv4(6379).await?;

        Ok(Self {
            url: format!("redis://127.0.0.1:{port}"),
            _container: Some(container),
        })
    }

    /// Redis connection URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Keeps either a CI-provided Qdrant URL or a locally spawned Qdrant container
/// alive.
pub struct QdrantService {
    url: String,
    _container: Option<ContainerAsync<GenericImage>>,
}

impl QdrantService {
    /// Resolve Qdrant from `QDRANT_URL` or start a local ephemeral container.
    pub async fn discover() -> Result<Self> {
        if let Some(url) = env_url("QDRANT_URL") {
            return Ok(Self {
                url,
                _container: None,
            });
        }

        let container = GenericImage::new("qdrant/qdrant", "v1.17.0")
            .with_exposed_port(6334.into())
            .with_wait_for(WaitFor::message_on_stderr("gRPC listening"))
            .start()
            .await?;
        let port = container.get_host_port_ipv4(6334).await?;

        Ok(Self {
            url: format!("http://127.0.0.1:{port}"),
            _container: Some(container),
        })
    }

    /// Qdrant base URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Build a unique test identifier with a stable human-readable prefix.
#[must_use]
pub fn unique_name(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}

fn env_url(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
