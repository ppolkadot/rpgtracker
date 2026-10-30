use serde::Deserialize;
use std::{collections::HashMap, env};

const DEFAULT_CONFIG_PATH: &str = "config.json";
const DEFAULT_COOLDOWN_MINUTES: u64 = 15;

#[derive(Deserialize, Clone)]
pub struct AppConfig {
    pub bot_name: String,
    pub bot_avatar: String,
    #[serde(default = "default_cooldown_minutes")]
    pub cooldown_minutes: u64,
    pub streamers: Vec<StreamerConfig>,
}

#[derive(Deserialize, Clone)]
pub struct StreamerConfig {
    pub username: String,
    pub channel_id: u64,
    pub ping_message: String,
    pub default_color: u32,
    pub custom_category: Option<String>,
    #[serde(default)]
    pub allowed_categories: Vec<String>,
    #[serde(default)]
    pub fields: Vec<EmbedField>,
    #[serde(default)]
    pub category_overrides: HashMap<String, CategoryStyle>,
}

#[derive(Deserialize, Clone)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

#[derive(Deserialize, Clone)]
pub struct CategoryStyle {
    pub label: String,
    pub color: u32,
}

fn default_cooldown_minutes() -> u64 {
    DEFAULT_COOLDOWN_MINUTES
}

/// Reads the config from `CONFIG_PATH`, defaulting to `config.json`.
pub async fn load_config() -> Result<AppConfig, String> {
    let path = env::var("CONFIG_PATH").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read {path}: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("Invalid JSON in {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let cfg: AppConfig = serde_json::from_str(include_str!("../config.example.json")).unwrap();
        assert_eq!(cfg.cooldown_minutes, 15);
        assert_eq!(cfg.streamers.len(), 1);
    }

    #[test]
    fn cooldown_defaults_when_missing() {
        let raw = r#"{"bot_name":"b","bot_avatar":"a","streamers":[]}"#;
        let cfg: AppConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.cooldown_minutes, DEFAULT_COOLDOWN_MINUTES);
    }
}
