//! Discord WebSocket gateway loop.
//!
//! The gateway is decomposed into three focused functions:
//! - [`run_gateway`]: outer reconnect loop with backoff
//! - [`connect_and_handshake`]: WebSocket connect + Resume/Identify handshake
//! - [`run_message_loop`]: inner event dispatch loop

use std::{collections::HashMap, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use orka_contracts::{
    CommandContent, InboundInteraction, InteractionContent, MediaAttachment, PlatformContext,
    SenderInfo, TraceContext, TrustLevel,
};
use orka_core::{
    InteractionSink, MessageId, SecretStr,
    types::{SessionId, backoff_delay},
};
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::{
    api,
    types::{GatewayEvent, ResumeState},
};

/// Shared, immutable gateway resources threaded through all helpers.
struct GatewayContext<'a> {
    bot_token: &'a Arc<SecretStr>,
    sessions: &'a Arc<Mutex<HashMap<String, SessionId>>>,
    http_client: &'a Client,
    sink: &'a InteractionSink,
    trust_level: TrustLevel,
}

/// Outcome of one WebSocket connection attempt or message-loop iteration.
enum ConnectionOutcome {
    /// The loop should reconnect (transient error, op 7/9, or WS close).
    Reconnect,
    /// The shutdown signal was received — stop the gateway entirely.
    Shutdown,
}

/// Run the Discord WebSocket gateway event loop until shutdown is signalled.
///
/// This function is designed to be spawned with `tokio::spawn`.
pub(crate) async fn run_gateway(
    initial_ws_url: String,
    bot_token: Arc<SecretStr>,
    sessions: Arc<Mutex<HashMap<String, SessionId>>>,
    http_client: Client,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    sink: InteractionSink,
    trust_level: TrustLevel,
) {
    let mut reconnect_count: u32 = 0;
    let mut resume = ResumeState {
        session_id: None,
        resume_gateway_url: None,
        sequence: None,
    };

    let ctx = GatewayContext {
        bot_token: &bot_token,
        sessions: &sessions,
        http_client: &http_client,
        sink: &sink,
        trust_level,
    };

    loop {
        if shutdown_rx.try_recv().is_ok() {
            info!("Discord adapter shutting down");
            break;
        }
        if reconnect_count > 0 {
            let delay = backoff_delay(reconnect_count - 1, 1, 60);
            warn!(attempt = reconnect_count, ?delay, "Discord reconnecting");
            tokio::time::sleep(delay).await;
        }
        let ws_url = resume.resume_gateway_url.as_ref().map_or_else(
            || initial_ws_url.clone(),
            |u| format!("{u}/?v=10&encoding=json"),
        );
        match connect_and_run(&ws_url, &ctx, &mut shutdown_rx, &mut resume, &mut reconnect_count).await {
            ConnectionOutcome::Reconnect => {}
            ConnectionOutcome::Shutdown => break,
        }
    }
}

/// Open one WebSocket connection, complete the handshake, run the message
/// loop, and return the outcome.
async fn connect_and_run(
    ws_url: &str,
    ctx: &GatewayContext<'_>,
    shutdown_rx: &mut tokio::sync::oneshot::Receiver<()>,
    resume: &mut ResumeState,
    reconnect_count: &mut u32,
) -> ConnectionOutcome {
    let (write, read, heartbeat_interval) =
        match connect_and_handshake(ws_url, ctx.bot_token, resume).await {
            Ok(parts) => parts,
            Err(outcome) => {
                *reconnect_count = reconnect_count.saturating_add(1);
                return outcome;
            }
        };
    *reconnect_count = 0;

    // Shared sequence number for the heartbeat task.
    let sequence_shared = Arc::new(Mutex::new(resume.sequence));
    let sequence_hb = sequence_shared.clone();
    let write = Arc::new(Mutex::new(write));
    let write_hb = write.clone();

    let heartbeat_handle = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval));
        loop {
            interval.tick().await;
            let seq = *sequence_hb.lock().await;
            let hb = serde_json::json!({
                "op": 1,
                "d": seq.map_or(serde_json::Value::Null, serde_json::Value::from)
            });
            let mut w = write_hb.lock().await;
            if w.send(tokio_tungstenite::tungstenite::Message::Text(hb.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let outcome =
        run_message_loop(read, ctx, shutdown_rx, resume, &sequence_shared, &heartbeat_handle).await;
    heartbeat_handle.abort();
    if matches!(outcome, ConnectionOutcome::Reconnect) {
        *reconnect_count = reconnect_count.saturating_add(1);
    }
    outcome
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;
type WsWriter = futures_util::stream::SplitSink<
    WsStream,
    tokio_tungstenite::tungstenite::Message,
>;
type WsReader = futures_util::stream::SplitStream<WsStream>;

/// Connect to the Discord WebSocket gateway and complete the handshake.
///
/// Returns `(write_half, read_half, heartbeat_interval_ms)` on success, or a
/// [`ConnectionOutcome`] if the connection or handshake fails.
async fn connect_and_handshake(
    ws_url: &str,
    bot_token: &Arc<SecretStr>,
    resume: &ResumeState,
) -> Result<(WsWriter, WsReader, u64), ConnectionOutcome> {
    let ws_stream = match tokio_tungstenite::connect_async(ws_url).await {
        Ok((stream, _)) => stream,
        Err(e) => {
            error!(%e, "failed to connect to Discord gateway");
            return Err(ConnectionOutcome::Reconnect);
        }
    };

    let (mut write, mut read) = ws_stream.split();

    // Read Hello (op 10)
    let heartbeat_interval = match read.next().await {
        Some(Ok(msg)) => serde_json::from_str::<GatewayEvent>(msg.to_text().unwrap_or("{}"))
            .ok()
            .and_then(|e| e.d?.get("heartbeat_interval")?.as_u64())
            .unwrap_or(41250),
        _ => 41250,
    };

    // Resume (op 6) or Identify (op 2)
    let handshake = if let (Some(sid), Some(seq)) =
        (resume.session_id.as_deref(), resume.sequence)
    {
        serde_json::json!({
            "op": 6,
            "d": { "token": bot_token.expose(), "session_id": sid, "seq": seq }
        })
    } else {
        serde_json::json!({
            "op": 2,
            "d": {
                "token": bot_token.expose(),
                "intents": 33280, // GUILDS | GUILD_MESSAGES | MESSAGE_CONTENT
                "properties": { "os": "linux", "browser": "orka", "device": "orka" }
            }
        })
    };

    if let Err(e) = write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            handshake.to_string().into(),
        ))
        .await
    {
        error!(%e, "failed to send Discord handshake");
        return Err(ConnectionOutcome::Reconnect);
    }

    Ok((write, read, heartbeat_interval))
}

/// Drive the inner WebSocket event loop for one connected session.
async fn run_message_loop(
    mut read: WsReader,
    ctx: &GatewayContext<'_>,
    shutdown_rx: &mut tokio::sync::oneshot::Receiver<()>,
    resume: &mut ResumeState,
    sequence_shared: &Arc<Mutex<Option<u64>>>,
    heartbeat_handle: &tokio::task::JoinHandle<()>,
) -> ConnectionOutcome {
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                info!("Discord adapter shutting down");
                return ConnectionOutcome::Shutdown;
            }
            msg = read.next() => match msg {
                Some(Ok(ws_msg)) => {
                    let text = ws_msg.to_text().unwrap_or("{}");
                    let Ok(event) = serde_json::from_str::<GatewayEvent>(text) else { continue };

                    if let Some(s) = event.s {
                        resume.sequence = Some(s);
                        *sequence_shared.lock().await = Some(s);
                    }

                    match event.op {
                        0 => {
                            let Some(ref d) = event.d else { continue };
                            match event.t.as_deref() {
                                Some("READY") => {
                                    resume.session_id = d["session_id"].as_str().map(String::from);
                                    resume.resume_gateway_url = d["resume_gateway_url"].as_str().map(String::from);
                                    info!("Discord READY");
                                }
                                Some("MESSAGE_CREATE") => {
                                    if handle_message_create(d, ctx.sessions, ctx.sink, heartbeat_handle, ctx.trust_level).await {
                                        return ConnectionOutcome::Shutdown;
                                    }
                                }
                                Some("INTERACTION_CREATE") => {
                                    if handle_interaction_create(d, ctx.sessions, ctx.http_client, ctx.bot_token, ctx.sink, heartbeat_handle, ctx.trust_level).await {
                                        return ConnectionOutcome::Shutdown;
                                    }
                                }
                                _ => {}
                            }
                        }
                        7 => {
                            warn!("Discord requested reconnect (op 7)");
                            return ConnectionOutcome::Reconnect;
                        }
                        9 => {
                            let resumable = event.d.as_ref().and_then(serde_json::Value::as_bool).unwrap_or(false);
                            warn!(resumable, "Discord Invalid Session (op 9)");
                            if !resumable {
                                resume.session_id = None;
                                resume.resume_gateway_url = None;
                                resume.sequence = None;
                            }
                            return ConnectionOutcome::Reconnect;
                        }
                        _ => {}
                    }
                }
                Some(Err(e)) => {
                    error!(%e, "Discord WebSocket error");
                    return ConnectionOutcome::Reconnect;
                }
                None => {
                    warn!("Discord WebSocket closed");
                    return ConnectionOutcome::Reconnect;
                }
            }
        }
    }
}

/// Handle a `MESSAGE_CREATE` dispatch event.
///
/// Returns `true` if the sink is closed and the gateway loop should terminate.
async fn handle_message_create(
    d: &serde_json::Value,
    sessions: &Arc<Mutex<HashMap<String, SessionId>>>,
    sink: &InteractionSink,
    heartbeat_handle: &tokio::task::JoinHandle<()>,
    trust_level: TrustLevel,
) -> bool {
    let is_bot = d["author"]["bot"].as_bool().unwrap_or(false);
    if is_bot {
        return false;
    }

    let channel_id = d["channel_id"].as_str().unwrap_or("");
    let session_id = {
        let mut s = sessions.lock().await;
        *s.entry(channel_id.to_string()).or_insert_with(SessionId::new)
    };
    let chat_type = if d.get("guild_id").and_then(|v| v.as_str()).is_some() {
        "group"
    } else {
        "direct"
    };

    if let Some(atts) = d["attachments"].as_array() {
        for att in atts {
            let interaction = InboundInteraction {
                id: MessageId::new().as_uuid(),
                source_channel: "discord".into(),
                session_id: session_id.as_uuid(),
                timestamp: chrono::Utc::now(),
                content: InteractionContent::Media(MediaAttachment {
                    mime_type: att["content_type"]
                        .as_str()
                        .unwrap_or("application/octet-stream")
                        .to_string(),
                    url: att["url"].as_str().unwrap_or("").to_string(),
                    filename: att["filename"].as_str().map(String::from),
                    caption: None,
                    size_bytes: att["size"].as_u64(),
                    data_base64: None,
                }),
                context: PlatformContext {
                    sender: SenderInfo {
                        platform_user_id: d["author"]["id"].as_str().map(String::from),
                        display_name: d["author"]["username"].as_str().map(String::from),
                        user_id: None,
                    },
                    chat_id: Some(channel_id.to_string()),
                    interaction_kind: Some(chat_type.into()),
                    guild_id: d["guild_id"].as_str().map(String::from),
                    trust_level: Some(trust_level),
                    ..Default::default()
                },
                trace: TraceContext::default(),
            };
            if sink.send(interaction).await.is_err() {
                heartbeat_handle.abort();
                return true;
            }
        }
    }

    let content = d["content"].as_str().unwrap_or("");
    if content.is_empty() {
        return false;
    }

    let interaction = InboundInteraction {
        id: MessageId::new().as_uuid(),
        source_channel: "discord".into(),
        session_id: session_id.as_uuid(),
        timestamp: chrono::Utc::now(),
        content: InteractionContent::Text(content.to_string()),
        context: PlatformContext {
            sender: SenderInfo {
                platform_user_id: d["author"]["id"].as_str().map(String::from),
                display_name: d["author"]["username"].as_str().map(String::from),
                user_id: None,
            },
            chat_id: Some(channel_id.to_string()),
            interaction_kind: Some(chat_type.into()),
            guild_id: d["guild_id"].as_str().map(String::from),
            trust_level: Some(trust_level),
            ..Default::default()
        },
        trace: TraceContext::default(),
    };

    if sink.send(interaction).await.is_err() {
        debug!("sink closed, stopping Discord listener");
        heartbeat_handle.abort();
        return true;
    }

    false
}

/// Handle an `INTERACTION_CREATE` dispatch event (`APPLICATION_COMMAND`, type 2).
///
/// Returns `true` if the sink is closed and the gateway loop should terminate.
async fn handle_interaction_create(
    d: &serde_json::Value,
    sessions: &Arc<Mutex<HashMap<String, SessionId>>>,
    http_client: &Client,
    bot_token: &Arc<SecretStr>,
    sink: &InteractionSink,
    heartbeat_handle: &tokio::task::JoinHandle<()>,
    trust_level: TrustLevel,
) -> bool {
    if d["type"].as_u64() != Some(2) {
        return false;
    }

    let interaction_id = d["id"].as_str().unwrap_or("").to_string();
    let interaction_token = d["token"].as_str().unwrap_or("").to_string();

    // ACK immediately to prevent Discord "interaction failed" after 3 s.
    {
        let client = http_client.clone();
        let token = Arc::clone(bot_token);
        let iid = interaction_id.clone();
        let itok = interaction_token.clone();
        tokio::spawn(async move {
            let ack_url = api::api_url(&format!("/interactions/{iid}/{itok}/callback"));
            let ack_body = serde_json::json!({"type": 5});
            if let Err(e) = client
                .post(&ack_url)
                .header("Authorization", format!("Bot {}", token.expose()))
                .json(&ack_body)
                .send()
                .await
            {
                warn!(%e, "Discord: failed to ACK interaction");
            }
        });
    }

    let channel_id = d["channel_id"].as_str().unwrap_or("");
    let cmd_name = d["data"]["name"].as_str().unwrap_or("").to_string();
    let mut args = HashMap::new();
    if let Some(opts) = d["data"]["options"].as_array() {
        for opt in opts {
            if let Some(name) = opt["name"].as_str() {
                args.insert(name.to_string(), opt["value"].clone());
            }
        }
    }

    let session_id = {
        let mut s = sessions.lock().await;
        *s.entry(channel_id.to_string()).or_insert_with(SessionId::new)
    };

    let interaction = InboundInteraction {
        id: MessageId::new().as_uuid(),
        source_channel: "discord".into(),
        session_id: session_id.as_uuid(),
        timestamp: chrono::Utc::now(),
        content: InteractionContent::Command(CommandContent { name: cmd_name, args }),
        context: PlatformContext {
            sender: SenderInfo {
                platform_user_id: d["member"]["user"]["id"]
                    .as_str()
                    .or_else(|| d["user"]["id"].as_str())
                    .map(String::from),
                display_name: d["member"]["user"]["username"]
                    .as_str()
                    .or_else(|| d["user"]["username"].as_str())
                    .map(String::from),
                user_id: None,
            },
            chat_id: Some(channel_id.to_string()),
            interaction_kind: Some("command".into()),
            guild_id: d["guild_id"].as_str().map(String::from),
            trust_level: Some(trust_level),
            extensions: [
                ("discord_interaction_id".into(), d["id"].clone()),
                ("discord_interaction_token".into(), d["token"].clone()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        trace: TraceContext::default(),
    };

    if sink.send(interaction).await.is_err() {
        heartbeat_handle.abort();
        return true;
    }

    false
}
