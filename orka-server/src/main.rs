use orka_core::config::OrkaConfig;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = OrkaConfig::load(None).expect("failed to load configuration");

    info!(?config, "Orka server starting");
}
