//! Platform adapter startup: inbound bridge and parallel initialization.

use std::sync::Arc;

use orka_config::OrkaConfig;
use orka_core::{
    Envelope, StreamRegistry,
    traits::{ChannelAdapter, MessageBus, SecretManager},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Start an adapter: create inbound bridge (adapter sink → bus "inbound")
/// and return the adapter Arc.
///
/// If `workspace_name` is provided, it's injected as `workspace:name` metadata
/// on every inbound envelope so the worker can resolve the correct workspace.
pub(crate) async fn start_adapter(
    adapter: Arc<dyn ChannelAdapter>,
    bus: Arc<dyn MessageBus>,
    shutdown: CancellationToken,
    workspace_name: Option<String>,
) -> anyhow::Result<()> {
    let (sink_tx, mut sink_rx) = mpsc::channel::<Envelope>(256);
    adapter.start(sink_tx).await?;

    let bus_for_bridge = bus.clone();
    let cancel = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                msg = sink_rx.recv() => {
                    match msg {
                        Some(mut envelope) => {
                            if let Some(ref ws) = workspace_name {
                                envelope.metadata.entry("workspace:name".to_string())
                                    .or_insert_with(|| serde_json::json!(ws));
                            }
                            if let Err(e) = bus_for_bridge.publish("inbound", &envelope).await {
                                error!(%e, "failed to publish inbound envelope to bus");
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });
    Ok(())
}

/// Arguments for starting all platform adapters.
pub(crate) struct AdapterStartArgs {
    pub secrets: Arc<dyn SecretManager>,
    pub bus: Arc<dyn MessageBus>,
    pub shutdown: CancellationToken,
    pub memory: Arc<dyn orka_core::traits::MemoryStore>,
    pub auth_layer: Option<orka_auth::AuthLayer>,
    pub stream_registry: StreamRegistry,
    pub config: OrkaConfig,
}

/// Start the custom adapter and all optional platform adapters.
///
/// Returns the custom adapter separately (needed for routing) plus a vec of
/// all adapters (including custom) for outbound routing and command
/// registration.
pub(crate) async fn start_all_adapters(
    args: AdapterStartArgs,
) -> anyhow::Result<(Arc<dyn ChannelAdapter>, Vec<Arc<dyn ChannelAdapter>>)> {
    let AdapterStartArgs {
        secrets,
        bus,
        shutdown,
        memory,
        auth_layer,
        stream_registry,
        config,
    } = args;

    let mut adapters: Vec<Arc<dyn ChannelAdapter>> = Vec::new();

    // Custom adapter (always started)
    let adapter_config = config.adapters.custom.clone().unwrap_or_default();
    let custom_adapter: Arc<dyn ChannelAdapter> = Arc::new(
        orka_adapter_custom::CustomAdapter::new(adapter_config, auth_layer, stream_registry),
    );
    let custom_ws = config
        .adapters
        .custom
        .as_ref()
        .and_then(|c| c.workspace.clone());
    start_adapter(
        custom_adapter.clone(),
        bus.clone(),
        shutdown.clone(),
        custom_ws,
    )
    .await?;
    adapters.push(custom_adapter.clone());
    info!("custom adapter started");

    // Optional platform adapters (started in parallel)
    #[cfg(feature = "telegram")]
    let tg_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let tg_config = config.adapters.telegram.clone();
        let tg_memory = memory.clone();
        async move {
            let tg_config = tg_config.as_ref()?;
            let secret_name = tg_config
                .bot_token_secret
                .as_deref()
                .unwrap_or("telegram_bot_token");
            match secrets.get_secret(secret_name).await {
                Ok(secret) => {
                    let token = secret.expose_str().unwrap_or("").to_string();
                    if token.is_empty() {
                        warn!("telegram bot token is empty, adapter disabled");
                        return None;
                    }
                    let tg: Arc<dyn ChannelAdapter> = Arc::new(
                        orka_adapter_telegram::TelegramAdapter::new(tg_config.clone(), token)
                            .with_memory(tg_memory),
                    );
                    if let Err(e) =
                        start_adapter(tg.clone(), bus, shutdown, tg_config.workspace.clone()).await
                    {
                        warn!(%e, "failed to start telegram adapter");
                        return None;
                    }
                    info!("telegram adapter started");
                    Some(tg)
                }
                Err(e) => {
                    warn!(%e, "failed to load telegram bot token, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "telegram"))]
    let tg_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    #[cfg(feature = "discord")]
    let dc_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let dc_config = config.adapters.discord.clone();
        async move {
            let dc_config = dc_config.as_ref()?;
            let secret_name = dc_config
                .bot_token_secret
                .as_deref()
                .unwrap_or("discord_bot_token");
            match secrets.get_secret(secret_name).await {
                Ok(secret) => {
                    let token = secret.expose_str().unwrap_or("").to_string();
                    if token.is_empty() {
                        warn!("discord bot token is empty, adapter disabled");
                        return None;
                    }
                    let dc: Arc<dyn ChannelAdapter> =
                        Arc::new(orka_adapter_discord::DiscordAdapter::new(token, None));
                    if let Err(e) =
                        start_adapter(dc.clone(), bus, shutdown, dc_config.workspace.clone()).await
                    {
                        warn!(%e, "failed to start discord adapter");
                        return None;
                    }
                    info!("discord adapter started");
                    Some(dc)
                }
                Err(e) => {
                    warn!(%e, "failed to load discord bot token, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "discord"))]
    let dc_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    #[cfg(feature = "slack")]
    let slack_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let slack_config = config.adapters.slack.clone();
        async move {
            let slack_config = slack_config.as_ref()?;
            let secret_name = slack_config
                .bot_token_secret
                .as_deref()
                .unwrap_or("slack_bot_token");
            match secrets.get_secret(secret_name).await {
                Ok(secret) => {
                    let token = secret.expose_str().unwrap_or("").to_string();
                    if token.is_empty() {
                        warn!("slack bot token is empty, adapter disabled");
                        return None;
                    }
                    let slack: Arc<dyn ChannelAdapter> = Arc::new(
                        orka_adapter_slack::SlackAdapter::new(token, slack_config.port),
                    );
                    if let Err(e) =
                        start_adapter(slack.clone(), bus, shutdown, slack_config.workspace.clone())
                            .await
                    {
                        warn!(%e, "failed to start slack adapter");
                        return None;
                    }
                    info!(port = slack_config.port, "slack adapter started");
                    Some(slack)
                }
                Err(e) => {
                    warn!(%e, "failed to load slack bot token, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "slack"))]
    let slack_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    #[cfg(feature = "whatsapp")]
    let wa_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let wa_config = config.adapters.whatsapp.clone();
        async move {
            let wa_config = wa_config.as_ref()?;
            let access_secret = wa_config
                .access_token_secret
                .as_deref()
                .unwrap_or("whatsapp_access_token");
            let verify_secret = wa_config.verify_token.clone().unwrap_or_default();
            let phone_id = wa_config.phone_number_id.clone().unwrap_or_default();
            match secrets.get_secret(access_secret).await {
                Ok(access) => {
                    let access_token = access.expose_str().unwrap_or("").to_string();
                    let verify_token = verify_secret;
                    if access_token.is_empty() || phone_id.is_empty() {
                        warn!(
                            "whatsapp access token or phone_number_id is empty, adapter disabled"
                        );
                        return None;
                    }
                    let wa: Arc<dyn ChannelAdapter> =
                        Arc::new(orka_adapter_whatsapp::WhatsAppAdapter::new(
                            access_token,
                            phone_id,
                            verify_token,
                            wa_config.port,
                        ));
                    if let Err(e) =
                        start_adapter(wa.clone(), bus, shutdown, wa_config.workspace.clone()).await
                    {
                        warn!(%e, "failed to start whatsapp adapter");
                        return None;
                    }
                    info!(port = wa_config.port, "whatsapp adapter started");
                    Some(wa)
                }
                Err(e) => {
                    warn!(%e, "failed to load whatsapp secrets, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "whatsapp"))]
    let wa_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    let (tg, dc, slack, wa) = tokio::join!(tg_fut, dc_fut, slack_fut, wa_fut);
    adapters.extend(tg);
    adapters.extend(dc);
    adapters.extend(slack);
    adapters.extend(wa);

    Ok((custom_adapter, adapters))
}
