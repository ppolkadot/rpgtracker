use crate::{
    config::{AppConfig, StreamerConfig},
    kick::Livestream,
};
use reqwest::{Client, StatusCode, header::RETRY_AFTER};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{error, info, warn};

// Discord's documented webhook limits.
const MAX_CONTENT: usize = 2000;
const MAX_TITLE: usize = 256;
const MAX_FIELDS: usize = 25;
const MAX_FIELD_NAME: usize = 256;
const MAX_FIELD_VALUE: usize = 1024;

const MAX_ATTEMPTS: u64 = 3;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
const DEFAULT_CATEGORY: &str = "Just Chatting";

pub fn apply_placeholders(
    template: &str,
    username: &str,
    url: &str,
    title: &str,
    category: &str,
    role_ping: &str,
) -> String {
    template
        .replace("{username}", username)
        .replace("{url}", url)
        .replace("{title}", title)
        .replace("{category}", category)
        .replace("{role_ping}", role_ping)
        .trim()
        .to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// The first Kick category of the stream, if the event carried one.
pub fn raw_category(live: Option<&Livestream>) -> Option<&str> {
    live.and_then(|l| l.categories.as_ref())
        .and_then(|c| c.first())
        .map(|c| c.name.as_str())
}

/// An empty `allowed_categories` allows everything, as does an event without a
/// category (better a possibly unwanted alert than a missed one). Otherwise match
/// case-insensitively.
pub fn is_allowed(streamer: &StreamerConfig, raw_category: Option<&str>) -> bool {
    let Some(category) = raw_category else {
        return true;
    };
    streamer.allowed_categories.is_empty()
        || streamer
            .allowed_categories
            .iter()
            .any(|c| c.eq_ignore_ascii_case(category))
}

pub fn resolve_category(streamer: &StreamerConfig, raw_category: &str) -> (String, u32) {
    let wanted = raw_category.to_lowercase();
    if let Some((_, style)) = streamer
        .category_overrides
        .iter()
        .find(|(name, _)| name.to_lowercase() == wanted)
    {
        return (style.label.clone(), style.color);
    }
    match &streamer.custom_category {
        Some(custom) => (custom.clone(), streamer.default_color),
        None => (raw_category.to_string(), streamer.default_color),
    }
}

pub fn build_payload(
    role_id: &str,
    cfg: &AppConfig,
    streamer: &StreamerConfig,
    live: Option<&Livestream>,
    now_secs: u64,
) -> Value {
    let (category, color) =
        resolve_category(streamer, raw_category(live).unwrap_or(DEFAULT_CATEGORY));

    let title = live
        .and_then(|l| l.session_title.clone())
        .unwrap_or_else(|| "Live on Kick!".to_string());

    let stream_url = format!("https://kick.com/{}", streamer.username);
    let role_ping = if role_id.is_empty() {
        String::new()
    } else {
        format!("<@&{role_id}>")
    };
    let render = |template: &str| {
        apply_placeholders(
            template,
            &streamer.username,
            &stream_url,
            &title,
            &category,
            &role_ping,
        )
    };

    let content = truncate(&render(&streamer.ping_message), MAX_CONTENT);

    // Discord rejects empty names/values, so drop fields that render to nothing.
    let fields: Vec<Value> = streamer
        .fields
        .iter()
        .map(|f| (render(&f.name), render(&f.value), f.inline))
        .filter(|(name, value, _)| !name.is_empty() && !value.is_empty())
        .take(MAX_FIELDS)
        .map(|(name, value, inline)| {
            json!({
                "name": truncate(&name, MAX_FIELD_NAME),
                "value": truncate(&value, MAX_FIELD_VALUE),
                "inline": inline
            })
        })
        .collect();

    let mut embed = json!({
        "title": truncate(&title, MAX_TITLE),
        "url": stream_url,
        "color": color,
        "fields": fields,
        "footer": { "text": "Kick Stream Monitor" }
    });

    if let Some(thumb_url) = live
        .and_then(|l| l.thumbnail.as_ref())
        .and_then(|t| t.url.as_ref())
    {
        embed["image"] = json!({ "url": format!("{thumb_url}?t={now_secs}") });
    }

    json!({
        "username": cfg.bot_name,
        "avatar_url": cfg.bot_avatar,
        "content": content,
        "embeds": [embed]
    })
}

pub async fn send_webhook(
    http: &Client,
    webhook_url: &str,
    role_id: &str,
    cfg: &AppConfig,
    streamer: &StreamerConfig,
    live: Option<Livestream>,
) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let payload = build_payload(role_id, cfg, streamer, live.as_ref(), now_secs);

    for attempt in 1..=MAX_ATTEMPTS {
        let mut delay = Duration::from_secs(2 * attempt);

        match http.post(webhook_url).json(&payload).send().await {
            Ok(res) if res.status().is_success() => {
                info!("Sent Discord alert for {}", streamer.username);
                return true;
            }
            Ok(res) => {
                let status = res.status();
                if let Some(secs) = res
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|s| s.is_finite() && *s >= 0.0)
                {
                    delay = Duration::from_secs_f64(secs);
                }
                let body = res.text().await.unwrap_or_default();
                error!("Discord error ({status}): {body}");
                if status != StatusCode::TOO_MANY_REQUESTS && !status.is_server_error() {
                    return false;
                }
            }
            Err(e) => error!("Failed to send webhook: {}", e.without_url()),
        }

        if attempt < MAX_ATTEMPTS {
            warn!(
                "Retrying Discord webhook in {:?}",
                delay.min(MAX_RETRY_DELAY)
            );
            sleep(delay.min(MAX_RETRY_DELAY)).await;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CategoryStyle;
    use std::collections::HashMap;

    fn streamer() -> StreamerConfig {
        StreamerConfig {
            username: "cellbit".into(),
            channel_id: 1,
            ping_message: "{role_ping} {username} is live: {url}".into(),
            default_color: 100,
            custom_category: None,
            allowed_categories: vec![],
            fields: vec![],
            category_overrides: HashMap::from([(
                "Tabletop RPGs".to_string(),
                CategoryStyle {
                    label: "RPG time".into(),
                    color: 200,
                },
            )]),
        }
    }

    fn app_config() -> AppConfig {
        AppConfig {
            bot_name: "bot".into(),
            bot_avatar: "avatar".into(),
            cooldown_minutes: 15,
            streamers: vec![],
        }
    }

    #[test]
    fn placeholders_are_replaced_and_trimmed() {
        let out = apply_placeholders(
            " {role_ping} {username} {category} ",
            "u",
            "url",
            "t",
            "c",
            "",
        );
        assert_eq!(out, "u c");
    }

    #[test]
    fn truncate_respects_char_limit() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ééééé", 3).chars().count(), 3);
    }

    #[test]
    fn override_matches_case_insensitively() {
        let (label, color) = resolve_category(&streamer(), "tabletop rpgs");
        assert_eq!((label.as_str(), color), ("RPG time", 200));
    }

    #[test]
    fn custom_category_replaces_name_without_override() {
        let mut s = streamer();
        s.custom_category = Some("Custom".into());
        let (label, color) = resolve_category(&s, "Chess");
        assert_eq!((label.as_str(), color), ("Custom", 100));
    }

    #[test]
    fn falls_back_to_kick_category() {
        let (label, color) = resolve_category(&streamer(), "Chess");
        assert_eq!((label.as_str(), color), ("Chess", 100));
    }

    #[test]
    fn allowed_categories_filter() {
        let mut s = streamer();
        assert!(is_allowed(&s, Some("Anything")));
        s.allowed_categories = vec!["Tabletop RPGs".into()];
        assert!(is_allowed(&s, Some("tabletop rpgs")));
        assert!(!is_allowed(&s, Some("Chess")));
    }

    #[test]
    fn missing_category_is_always_allowed() {
        let mut s = streamer();
        s.allowed_categories = vec!["Tabletop RPGs".into()];
        assert!(is_allowed(&s, None));
        assert_eq!(raw_category(None), None);
    }

    #[test]
    fn payload_enforces_discord_limits() {
        let mut s = streamer();
        s.ping_message = "x".repeat(MAX_CONTENT + 50);
        s.fields = (0..40)
            .map(|i| crate::config::EmbedField {
                name: format!("n{i}"),
                value: "v".repeat(2000),
                inline: true,
            })
            .collect();
        s.fields.push(crate::config::EmbedField {
            name: "".into(),
            value: "x".into(),
            inline: false,
        });
        let payload = build_payload("", &app_config(), &s, None, 0);
        assert_eq!(
            payload["content"].as_str().unwrap().chars().count(),
            MAX_CONTENT
        );
        let fields = payload["embeds"][0]["fields"].as_array().unwrap();
        assert_eq!(fields.len(), MAX_FIELDS);
        assert_eq!(
            fields[0]["value"].as_str().unwrap().chars().count(),
            MAX_FIELD_VALUE
        );
    }

    #[test]
    fn payload_pings_role_and_defaults_title() {
        let payload = build_payload("123", &app_config(), &streamer(), None, 0);
        assert_eq!(
            payload["content"],
            "<@&123> cellbit is live: https://kick.com/cellbit"
        );
        assert_eq!(payload["embeds"][0]["title"], "Live on Kick!");
    }
}
