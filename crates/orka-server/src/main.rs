mod adapters;
mod bootstrap;
mod env_watcher;
mod experience;
mod providers;
mod scheduler_adapter;
mod update;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    bootstrap::run().await
}
