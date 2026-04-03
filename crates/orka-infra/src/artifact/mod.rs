//! Artifact storage backed by Redis.
//!
//! Provides [`RedisArtifactStore`] and a factory for wiring it into the server
//! runtime.

/// Redis implementation of the artifact store.
pub mod redis_store;

use std::sync::Arc;

use orka_core::traits::ArtifactStore;

pub use self::redis_store::RedisArtifactStore;

/// Create an [`ArtifactStore`] from the given Redis URL.
pub fn create_artifact_store(redis_url: &str) -> orka_core::Result<Arc<dyn ArtifactStore>> {
    let store = RedisArtifactStore::new(redis_url)?;
    Ok(Arc::new(store))
}
