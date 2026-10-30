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

type Cooldowns = HashMap<u64, Instant>;

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
    let mut cooldowns = Cooldowns::new();
    let mut backoff = RECONNECT_BASE;

    loop {
        // Pick up config edits (including new streamers to subscribe to) on every reconnect,
        // but never let a bad edit take the bot down.
        match config::load_config().await {
            Ok(fresh) => cfg = fresh,
            Err(e) => warn!("{e}, keeping the previous config"),
        }

        info!("Connecting to Kick");
        match connect_async(kick::pusher_url()).await {
            Ok((ws, _)) => {
                backoff = RECONNECT_BASE;
                info!("Connected to Kick");
                let reason =
                    run_session(ws, &http, &webhook_url, &role_id, &cfg, &mut cooldowns).await;
                warn!("Session ended: {reason}");
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
    cooldowns: &mut Cooldowns,
) -> String
where
    S: Stream<Item = Result<Message, tungstenite::Error>>
        + Sink<Message, Error = tungstenite::Error>
        + Unpin,
{
    for streamer in &cfg.streamers {
        let sub = kick::subscribe_message(streamer.channel_id);
        if let Err(e) = ws.send(Message::Text(sub.to_string().into())).await {
            return format!(
                "failed to subscribe to channel.{}: {e}",
                streamer.channel_id
            );
        }
        info!(
            "Watching {} (channel.{})",
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
                    kick::LIVE_EVENT => {
                        handle_live(msg, http, webhook_url, role_id, cfg, cooldowns).await;
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
    cooldowns: &mut Cooldowns,
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

    let live = msg
        .data
        .and_then(|d| serde_json::from_str::<kick::LiveEventData>(&d).ok())
        .and_then(|e| e.livestream);

    let category = discord::raw_category(live.as_ref());
    if !discord::is_allowed(streamer, category) {
        info!(
            "Skipping {}: category {category:?} is not in allowed_categories",
            streamer.username
        );
        return;
    }

    let cooldown = Duration::from_secs(cfg.cooldown_minutes * 60);
    if cooldowns
        .get(&id)
        .is_some_and(|last| last.elapsed() < cooldown)
    {
        return;
    }

    info!("{} went LIVE", streamer.username);
    if discord::send_webhook(http, webhook_url, role_id, &cfg, streamer, live).await {
        cooldowns.insert(id, Instant::now());
    }
}
