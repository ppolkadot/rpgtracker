use serde::Deserialize;
use serde_json::{Value, json};

// Kick's public Pusher app key and cluster, as used by kick.com's own web client.
// This is not a secret. If alerts stop arriving, check whether Kick rotated these.
const PUSHER_APP_KEY: &str = "32cbd69e4b950bf97679";
const PUSHER_HOST: &str = "ws-us2.pusher.com";

pub const LIVE_EVENT: &str = r"App\Events\StreamerIsLive";

pub fn pusher_url() -> String {
    format!(
        "wss://{PUSHER_HOST}/app/{PUSHER_APP_KEY}?protocol=7&client=js&version=8.4.0-rc2&flash=false"
    )
}

pub fn subscribe_message(channel_id: u64) -> Value {
    json!({
        "event": "pusher:subscribe",
        "data": { "auth": "", "channel": format!("channel.{channel_id}") }
    })
}

pub fn pong_message() -> Value {
    json!({ "event": "pusher:pong", "data": {} })
}

pub fn channel_id(channel: &str) -> Option<u64> {
    channel.strip_prefix("channel.")?.parse().ok()
}

#[derive(Deserialize)]
pub struct PusherMessage {
    pub event: String,
    pub channel: Option<String>,
    pub data: Option<String>,
}

#[derive(Deserialize)]
pub struct LiveEventData {
    pub livestream: Option<Livestream>,
}

#[derive(Deserialize)]
pub struct Livestream {
    pub session_title: Option<String>,
    pub categories: Option<Vec<KickCategory>>,
    pub thumbnail: Option<Thumbnail>,
}

#[derive(Deserialize)]
pub struct KickCategory {
    pub name: String,
}

#[derive(Deserialize)]
pub struct Thumbnail {
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_channel_ids() {
        assert_eq!(channel_id("channel.42"), Some(42));
        assert_eq!(channel_id("channel.abc"), None);
        assert_eq!(channel_id("chatrooms.42"), None);
    }

    #[test]
    fn parses_live_event_payload() {
        let raw = r#"{"livestream":{"session_title":"hi","categories":[{"name":"Tabletop RPGs"}],"thumbnail":{"url":"https://x/y.jpg"}}}"#;
        let data: LiveEventData = serde_json::from_str(raw).unwrap();
        let live = data.livestream.unwrap();
        assert_eq!(live.session_title.as_deref(), Some("hi"));
        assert_eq!(live.categories.unwrap()[0].name, "Tabletop RPGs");
    }
}
