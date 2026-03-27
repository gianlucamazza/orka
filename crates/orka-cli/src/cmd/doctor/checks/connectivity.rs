use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::cmd::doctor::{
    CheckContext, DoctorCheck,
    types::{Category, CheckId, CheckMeta, CheckOutcome, Severity},
};

pub struct ConRedisReachable;
pub struct ConRedisVersion;
pub struct ConQdrantReachable;
pub struct ConQdrantVersion;

#[async_trait]
impl DoctorCheck for ConRedisReachable {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("CON-001"),
            category: Category::Connectivity,
            severity: Severity::Critical,
            name: "Redis reachable",
            description: "Redis must be reachable at the configured URL via PING.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(cfg) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };
        let url = cfg.redis.url.clone();

        let start = Instant::now();
        match redis_ping(&url, ctx.timeout).await {
            Ok(()) => {
                let ms = start.elapsed().as_millis();
                CheckOutcome::pass(format!("{url} ({ms}ms)"))
            }
            Err(e) => CheckOutcome::fail(format!("cannot connect to {url}: {e}")).with_hint(
                "Ensure Redis is running. Start it with `redis-server` or `docker run redis`.",
            ),
        }
    }

    fn explain(&self) -> &'static str {
        "Orka requires Redis 7+ for the message bus (Redis Streams), priority queue \
         (Sorted Sets), session store, memory store, and secret storage. \
         The URL is read from config.redis.url (default: redis://127.0.0.1:6379). \
         Override with ORKA__REDIS__URL environment variable."
    }
}

#[async_trait]
impl DoctorCheck for ConRedisVersion {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("CON-002"),
            category: Category::Connectivity,
            severity: Severity::Warning,
            name: "Redis version >= 7.0",
            description: "Orka requires Redis 7.0+ for Redis Streams features.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(cfg) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };
        let url = cfg.redis.url.clone();

        match redis_version(&url, ctx.timeout).await {
            Ok(version) => {
                let parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
                let major = parts.first().copied().unwrap_or(0);
                if major >= 7 {
                    CheckOutcome::pass(format!("v{version}"))
                } else {
                    CheckOutcome::fail(format!("v{version} — need >= 7.0"))
                        .with_hint("Upgrade Redis to 7.0 or later.")
                }
            }
            Err(e) => CheckOutcome::skip(format!("cannot query version: {e}")),
        }
    }
}

#[async_trait]
impl DoctorCheck for ConQdrantReachable {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("CON-003"),
            category: Category::Connectivity,
            severity: Severity::Error,
            name: "Qdrant reachable",
            description: "Qdrant must be reachable when knowledge.enabled = true.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if !config.knowledge.enabled {
            return CheckOutcome::skip("knowledge.enabled = false");
        }

        let url = config
            .knowledge
            .vector_store
            .url
            .clone()
            .unwrap_or_else(|| "http://localhost:6334".to_string());
        let start = Instant::now();

        match qdrant_ping(&url, ctx.timeout).await {
            Ok(()) => {
                let ms = start.elapsed().as_millis();
                CheckOutcome::pass(format!("{url} ({ms}ms)"))
            }
            Err(e) => CheckOutcome::fail(format!("cannot connect to {url}: {e}")).with_hint(
                "Ensure Qdrant is running. Start with `docker run -p 6334:6334 qdrant/qdrant`.",
            ),
        }
    }

    fn explain(&self) -> &'static str {
        "Qdrant is the vector store used by orka-knowledge for RAG (Retrieval-Augmented \
         Generation) and by orka-experience for storing learned principles. \
         Required only when knowledge.enabled = true in orka.toml. \
         Default URL: http://localhost:6334."
    }
}

#[async_trait]
impl DoctorCheck for ConQdrantVersion {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("CON-004"),
            category: Category::Connectivity,
            severity: Severity::Info,
            name: "Qdrant version",
            description: "Reports the connected Qdrant version (informational).",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if !config.knowledge.enabled {
            return CheckOutcome::skip("knowledge.enabled = false");
        }

        let url = config
            .knowledge
            .vector_store
            .url
            .clone()
            .unwrap_or_else(|| "http://localhost:6334".to_string());
        match qdrant_version(&url, ctx.timeout).await {
            Ok(v) => CheckOutcome::pass(format!("v{v}")),
            Err(e) => CheckOutcome::skip(format!("cannot query version: {e}")),
        }
    }
}

async fn redis_ping(
    url: &str,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = redis::Client::open(url)?;
    let mut conn = tokio::time::timeout(timeout, client.get_multiplexed_async_connection())
        .await
        .map_err(|_| "connection timed out")??;

    redis::cmd("PING").query_async::<String>(&mut conn).await?;
    Ok(())
}

async fn redis_version(
    url: &str,
    timeout: Duration,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = redis::Client::open(url)?;
    let mut conn = tokio::time::timeout(timeout, client.get_multiplexed_async_connection())
        .await
        .map_err(|_| "connection timed out")??;

    let info: String = redis::cmd("INFO")
        .arg("server")
        .query_async(&mut conn)
        .await?;

    for line in info.lines() {
        if let Some(ver) = line.strip_prefix("redis_version:") {
            return Ok(ver.trim().to_string());
        }
    }
    Err("redis_version not found in INFO output".into())
}

async fn qdrant_ping(
    url: &str,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = qdrant_client::Qdrant::from_url(url).build()?;
    tokio::time::timeout(timeout, client.health_check())
        .await
        .map_err(|_| "health check timed out")??;
    Ok(())
}

async fn qdrant_version(
    url: &str,
    timeout: Duration,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = qdrant_client::Qdrant::from_url(url).build()?;
    let response = tokio::time::timeout(timeout, client.health_check())
        .await
        .map_err(|_| "health check timed out")??;
    Ok(response.version)
}
