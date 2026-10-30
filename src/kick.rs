use serde::Deserialize;
use serde_json::{Value, json};

pub const LIVE_EVENT: &str = r"App\Events\StreamerIsLive";

pub async fn pusher_url(http: &reqwest::Client, channel_id: u64) -> Result<String, String> {
    let endpoint =
        format!("https://web.kick.com/api/v1/realtime/channels/{channel_id}/chat/connection");
    let response = http
        .post(endpoint)
        .json(&json!({
            "client": {"id": "00000000-0000-4000-8000-000000000000", "type": "web"},
            "capabilities": {"accepted_providers": [{"provider": "pusher"}]}
        }))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?
        .error_for_status()
        .map_err(|e| e.without_url().to_string())?;
    let value: Value = response
        .json()
        .await
        .map_err(|e| e.without_url().to_string())?;
    pusher_url_from_response(&value)
}

fn pusher_url_from_response(value: &Value) -> Result<String, String> {
    let credentials = value["data"]["connections"]
        .as_array()
        .and_then(|connections| connections.iter().find(|c| c["provider"] == "pusher"))
        .map(|c| &c["credentials"])
        .ok_or("Kick did not return Pusher credentials")?;
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    let key = credentials["app_key"]
        .as_str()
        .filter(|s| valid(s))
        .ok_or("invalid Pusher app key")?;
    let cluster = credentials["cluster"]
        .as_str()
        .filter(|s| valid(s))
        .ok_or("invalid Pusher cluster")?;
    Ok(format!(
        "wss://ws-{cluster}.pusher.com/app/{key}?protocol=7&client=js&version=8.4.0-rc2&flash=false"
    ))
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
    pub data: Option<Value>,
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
    fn connection_settings_use_kicks_key_and_cluster() {
        let response = json!({"data":{"connections":[{"provider":"pusher","credentials":{"app_key":"abc123","cluster":"us2"}}]}});
        let url = pusher_url_from_response(&response).unwrap();
        assert!(url.starts_with("wss://ws-us2.pusher.com/app/abc123?"));
        assert!(pusher_url_from_response(&json!({"data":{"connections":[]}})).is_err());
        let bad = json!({"data":{"connections":[{"provider":"pusher","credentials":{"app_key":"abc","cluster":"evil.example"}}]}});
        assert!(pusher_url_from_response(&bad).is_err());
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
