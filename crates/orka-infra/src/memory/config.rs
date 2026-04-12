use serde::Deserialize;

/// Memory backend options.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    /// Auto-detect based on environment.
    #[default]
    Auto,
    /// Redis backend.
    Redis,
    /// In-memory backend (ephemeral).
    Memory,
}

const fn default_max_entries() -> usize {
    1000
}

/// In-memory (Redis) memory store configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct MemoryConfig {
    /// Memory backend to use.
    #[serde(default)]
    pub backend: MemoryBackend,
    /// Maximum number of entries to keep in memory.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackend::default(),
            max_entries: default_max_entries(),
        }
    }
}

impl MemoryConfig {
    /// Validate memory configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.max_entries == 0 {
            return Err(orka_core::Error::Config(
                "memory.max_entries must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}
