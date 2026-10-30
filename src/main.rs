mod config;
mod discord;
mod kick;

use config::AppConfig;
use dotenvy::dotenv;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use kick::PusherMessage;
use reqwest::Client;
use std::{
    collections::HashMap,
    env,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, protocol::Message},
};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

const RECONNECT_BASE: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(5 * 60);
// Pusher pings idle connections every ~120s, so silence for longer means a dead socket.
const READ_TIMEOUT: Duration = Duration::from_secs(180);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

type Cooldowns = Arc<Mutex<HashMap<u64, Option<Instant>>>>;

#[tokio::main]
async fn main() {
    dotenv().ok();
    dotenvy::from_filename("secrets.env").ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let webhook_url = env::var("DISCORD_WEBHOOK_URL")
        .expect("Missing DISCORD_WEBHOOK_URL (set it in .env or secrets.env)");
    let role_id = env::var("ROLE_ID")
        .or_else(|_| env::var("PING_ROLE_ID"))
        .unwrap_or_default();

    let http = Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .expect("Failed to build HTTP client");

    let mut cfg = config::load_config()
        .await
        .expect("Could not load config on startup");
    let cooldowns = Cooldowns::default();
    let mut backoff = RECONNECT_BASE;

    loop {
        // Pick up config edits (including new streamers to subscribe to) on every reconnect,
        // but never let a bad edit take the bot down.
        match config::load_config().await {
            Ok(fresh) => cfg = fresh,
            Err(e) => warn!("{e}, keeping the previous config"),
        }

        let Some(first) = cfg.streamers.first() else {
            warn!("No streamers configured; retrying configuration in {backoff:?}");
            sleep(backoff).await;
            backoff = (backoff * 2).min(RECONNECT_MAX);
            continue;
        };
        let url = match kick::pusher_url(&http, first.channel_id).await {
            Ok(url) => url,
            Err(e) => {
                error!("Could not obtain Kick connection settings: {e}");
                sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX);
                continue;
            }
        };
        info!("Connecting to Kick");
        match connect_async(url).await {
            Ok((ws, _)) => {
                let started = Instant::now();
                let reason = run_session(ws, &http, &webhook_url, &role_id, &cfg, &cooldowns).await;
                warn!("Session ended: {reason}");
                if started.elapsed() >= Duration::from_secs(60) {
                    backoff = RECONNECT_BASE;
                }
            }
            Err(e) => error!("WebSocket connect error: {e}"),
        }

        warn!("Reconnecting in {backoff:?}");
        sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

async fn run_session<S>(
    mut ws: S,
    http: &Client,
    webhook_url: &str,
    role_id: &str,
    cfg: &AppConfig,
    cooldowns: &Cooldowns,
) -> String
where
    S: Stream<Item = Result<Message, tungstenite::Error>>
        + Sink<Message, Error = tungstenite::Error>
        + Unpin,
{
    let first_frame = match timeout(Duration::from_secs(15), ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        Ok(Some(Ok(Message::Close(frame)))) => return format!("closed by server: {frame:?}"),
        other => return format!("Pusher handshake failed: {other:?}"),
    };
    match serde_json::from_str::<PusherMessage>(&first_frame) {
        Ok(msg) if msg.event == "pusher:connection_established" => info!("Connected to Kick"),
        _ => return "Pusher did not confirm the connection".to_string(),
    }
    for streamer in &cfg.streamers {
        let sub = kick::subscribe_message(streamer.channel_id);
        if let Err(e) = ws.send(Message::Text(sub.to_string().into())).await {
            return format!(
                "failed to subscribe to channel.{}: {e}",
                streamer.channel_id
            );
        }
        info!(
            "Requested subscription for {} (channel.{})",
            streamer.username, streamer.channel_id
        );
    }

    loop {
        let frame = match timeout(READ_TIMEOUT, ws.next()).await {
            Err(_) => return format!("no data received for {}s", READ_TIMEOUT.as_secs()),
            Ok(None) => return "stream closed".to_string(),
            Ok(Some(Err(e))) => return format!("read error: {e}"),
            Ok(Some(Ok(frame))) => frame,
        };

        match frame {
            Message::Text(text) => {
                let Ok(msg) = serde_json::from_str::<PusherMessage>(&text) else {
                    continue;
                };
                match msg.event.as_str() {
                    "pusher:ping" => {
                        let pong = kick::pong_message();
                        if let Err(e) = ws.send(Message::Text(pong.to_string().into())).await {
                            return format!("failed to send pong: {e}");
                        }
                    }
                    "pusher:error" => warn!("Pusher error: {:?}", msg.data),
                    "pusher_internal:subscription_succeeded" => {
                        if let Some(channel) = msg.channel.as_deref() {
                            info!("Subscription confirmed: {channel}");
                        }
                    }
                    "pusher_internal:subscription_error" => {
                        warn!("Subscription failed for {:?}: {:?}", msg.channel, msg.data);
                    }
                    kick::LIVE_EVENT => {
                        let http = http.clone();
                        let webhook_url = webhook_url.to_owned();
                        let role_id = role_id.to_owned();
                        let cfg = cfg.clone();
                        let cooldowns = cooldowns.clone();
                        tokio::spawn(async move {
                            handle_live(msg, &http, &webhook_url, &role_id, &cfg, &cooldowns).await;
                        });
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                if let Err(e) = ws.send(Message::Pong(payload)).await {
                    return format!("failed to answer ping: {e}");
                }
            }
            Message::Close(frame) => return format!("closed by server: {frame:?}"),
            _ => {}
        }
    }
}

async fn handle_live(
    msg: PusherMessage,
    http: &Client,
    webhook_url: &str,
    role_id: &str,
    fallback_cfg: &AppConfig,
    cooldowns: &Cooldowns,
) {
    let Some(id) = msg.channel.as_deref().and_then(kick::channel_id) else {
        return;
    };

    // Fresh reload so config edits apply without a restart.
    let cfg = config::load_config().await.unwrap_or_else(|e| {
        warn!("{e}, falling back to the session config");
        fallback_cfg.clone()
    });

    let Some(streamer) = cfg.streamers.iter().find(|s| s.channel_id == id) else {
        return;
    };

    let parsed = match msg.data {
        Some(serde_json::Value::String(d)) => serde_json::from_str::<kick::LiveEventData>(&d),
        Some(d) => serde_json::from_value::<kick::LiveEventData>(d),
        None => {
            warn!("Live event for {} had no data", streamer.username);
            return;
        }
    };
    let live = match parsed {
        Ok(event) if event.livestream.is_some() => event.livestream,
        Ok(_) => {
            warn!("Live event for {} had no livestream", streamer.username);
            return;
        }
        Err(e) => {
            warn!("Invalid live event for {}: {e}", streamer.username);
            return;
        }
    };

    let category = discord::raw_category(live.as_ref());
    if !discord::is_allowed(streamer, category) {
        info!(
            "Skipping {}: category {category:?} is not in allowed_categories",
            streamer.username
        );
        return;
    }

    let cooldown = Duration::from_secs(cfg.cooldown_minutes * 60);
    {
        let mut state = cooldowns.lock().expect("cooldown lock poisoned");
        if state
            .get(&id)
            .is_some_and(|last| last.is_none_or(|last| last.elapsed() < cooldown))
        {
            return;
        }
        state.insert(id, None);
    }

    info!("{} went LIVE", streamer.username);
    let sent = discord::send_webhook(http, webhook_url, role_id, &cfg, streamer, live).await;
    let mut state = cooldowns.lock().expect("cooldown lock poisoned");
    if sent {
        state.insert(id, Some(Instant::now()));
    } else {
        state.remove(&id);
    }
}
