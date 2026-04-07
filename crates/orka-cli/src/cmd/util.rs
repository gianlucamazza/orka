//! Shared CLI utilities.

/// Convert the orka-config secret configuration into the runtime
/// `orka_secrets::SecretConfig` used to instantiate a [`SecretManager`].
///
/// [`SecretManager`]: orka_core::traits::SecretManager
pub fn runtime_secret_config(config: &orka_config::SecretConfig) -> orka_secrets::SecretConfig {
    let backend = match config.backend {
        orka_config::SecretBackend::Redis => orka_secrets::SecretBackend::Redis,
        orka_config::SecretBackend::File => orka_secrets::SecretBackend::File,
        _ => orka_secrets::SecretBackend::default(),
    };
    let mut runtime = orka_secrets::SecretConfig::default();
    runtime.backend = backend;
    runtime.file_path.clone_from(&config.file_path);
    runtime
        .encryption_key_path
        .clone_from(&config.encryption_key_path);
    runtime
        .encryption_key_env
        .clone_from(&config.encryption_key_env);
    runtime.redis.url.clone_from(&config.redis.url);
    runtime
}
