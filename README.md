# rpgtracker
dumb little bot to track cellbit lives - created for someone i like

all in.

[![CI](https://github.com/ppolkadot/rpgtracker/actions/workflows/ci.yml/badge.svg)](https://github.com/ppolkadot/rpgtracker/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A small Rust service that watches Kick streamers and posts a Discord alert the moment they go live.

## Contents

- [Features](#features)
- [How it works](#how-it-works)
- [Getting started](#getting-started)
- [Configuration](#configuration)
- [Running](#running)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [License](#license)

## Features

- **Real-time alerts.** Subscribes to Kick's live event feed instead of polling.
- **Multiple streamers.** Watch as many channels as you like from one config file.
- **Customizable messages.** Per-streamer ping text, embed fields, colors and category labels, with placeholders for the stream title, category and link.
- **Category filtering and styling.** Only alert for the categories you care about, and give each one its own label and color.
- **Hot-reloaded config.** Edits to `config.json` apply to the next alert without a restart.
- **Resilient by default.**
  - Reconnects automatically with exponential backoff and detects silently dead connections.
  - Retries Discord rate limits and server errors.
  - Keeps running on the last good config if you save a broken one.
- **Duplicate protection.** A per-streamer cooldown stops repeated events from spamming the channel.

## How it works

```
Kick (Pusher WebSocket) ──► rpgtracker ──► Discord webhook
     StreamerIsLive event      filter, cooldown,      embed + role ping
                               format message
```

1. On startup the bot asks Kick for current Pusher connection settings, connects, and subscribes to each configured `channel_id`. The logs report connection and subscription confirmations separately.
2. When a `StreamerIsLive` event arrives, it reloads `config.json`, checks the category filter and the cooldown, and builds the message.
3. The message is posted to your Discord webhook. The cooldown only starts once Discord accepts it.

## Getting started

### Requirements

- A recent stable [Rust toolchain](https://rustup.rs) (the crate uses edition 2024, so Rust 1.85 or newer).
- A Discord webhook URL. Create one under *Channel settings → Integrations → Webhooks*.
- The Kick channel id of each streamer you want to track.

### Install

```sh
git clone https://github.com/ppolkadot/rpgtracker
cd rpgtracker
```

### Set up credentials

Copy the example environment file and fill in your webhook:

```sh
cp .env.example .env
```

Windows PowerShell: `Copy-Item .env.example .env`

### Set up the streamer config

```sh
cp config.example.json config.json
```

Then edit `config.json` (see [Configuration](#configuration)).

> `.env`, `secrets.env` and `config.json` are gitignored, so your webhook and settings stay on your machine.

## Configuration

### Environment variables

Loaded from `.env`, and also from `secrets.env` if present, in the working directory.

| Variable | Required | Default | Description |
|---|---|---|---|
| `DISCORD_WEBHOOK_URL` | yes | | Discord webhook the alerts are posted to |
| `ROLE_ID` | no | | Discord role id inserted by `{role_ping}` (`PING_ROLE_ID` is accepted as an alias) |
| `CONFIG_PATH` | no | `config.json` | Path to the streamer config |
| `RUST_LOG` | no | `info` | Log level: `error`, `warn`, `info`, `debug` or `trace` |

### Streamer config (`config.json`)

Top-level fields:

| Field | Type | Default | Description |
|---|---|---|---|
| `bot_name` | string | required | Name shown on the Discord message |
| `bot_avatar` | string | required | Avatar image URL shown on the Discord message |
| `cooldown_minutes` | number | `15` | Minimum minutes between alerts for the same streamer |
| `streamers` | array | required | Streamers to watch |

Each entry in `streamers`:

| Field | Type | Default | Description |
|---|---|---|---|
| `username` | string | required | Kick username, used to build the stream link |
| `channel_id` | number | required | Kick channel id the bot subscribes to |
| `ping_message` | string | required | Text posted above the embed |
| `default_color` | number | required | Embed color as a decimal integer |
| `custom_category` | string or `null` | `null` | Replaces the category name used by `{category}` |
| `allowed_categories` | array of strings | `[]` | Only alert for these Kick categories (case-insensitive). Empty allows all |
| `fields` | array | `[]` | Extra embed fields, each with `name`, `value` and `inline` |
| `category_overrides` | object | `{}` | Per Kick category, a `label` and `color` that replace the defaults |

An example with every field is in [`config.example.json`](config.example.json).

### Placeholders

Usable in `ping_message` and in the `name` and `value` of every field:

| Placeholder | Replaced with |
|---|---|
| `{username}` | The streamer's Kick username |
| `{url}` | Link to the stream, `https://kick.com/<username>` |
| `{title}` | The stream title |
| `{category}` | The category label (after overrides and `custom_category`) |
| `{role_ping}` | A mention of `ROLE_ID`, or nothing if it is not set |

### Behaviour details

- **Category precedence.** A matching `category_overrides` entry wins over `custom_category`, which wins over the name Kick reports. Matching is case-insensitive.
- **Events without a category.** If Kick sends no category, the alert is always sent, regardless of `allowed_categories`, and the label defaults to "Just Chatting".
- **Discord limits.** Message content is cut at 2000 characters, titles at 256, field names at 256 and field values at 1024. At most 25 fields are sent, and fields that render empty are dropped because Discord rejects them.
- **Embed budget.** The total text in the embed is limited to 6000 characters; later fields may be shortened or omitted.
- **Hot reload.** `config.json` is re-read on every live event, and again on every reconnect. A newly added streamer is subscribed to on the next reconnect. If the file is invalid at runtime, the bot logs a warning and keeps the last good config.
- **Delivery.** Rate limits (429) and server errors (5xx) are retried, for up to 3 attempts in total, honoring Discord's `Retry-After`. Other errors are logged and not retried.
- **Reconnects.** Connection drops reconnect with exponential backoff, from 5 seconds up to 5 minutes. A connection that goes silent for 3 minutes is treated as dead and re-established.
- **Outages and restarts.** The event feed has no replay in this application, so streams that start while disconnected can be missed. Cooldowns are in memory and reset when the process restarts.

## Running

```sh
cargo run --release
```

Or build the binary once and run it from the directory that contains your `.env` and `config.json` (or point `CONFIG_PATH` at the config):

```sh
cargo build --release
./target/release/rpgtrack        # target\release\rpgtrack.exe on Windows
```

The Windows build embeds `yamada.ico` as the executable icon.

The bot runs until stopped and logs to standard output. Set `RUST_LOG=debug` for more detail.

## Troubleshooting

| Symptom | Likely cause and fix |
|---|---|
| Panics on startup with `Missing DISCORD_WEBHOOK_URL` | The variable is not set. Put it in `.env` or `secrets.env` in the directory you run the bot from. |
| `Could not load config on startup` | `config.json` is missing or is not valid JSON. The error includes the path and the reason. |
| Connects but no alerts arrive | Check `channel_id`, make sure the category is not excluded by `allowed_categories`, and remember the cooldown. Run with `RUST_LOG=debug`. |
| `Could not obtain Kick connection settings` | Kick's realtime connection endpoint may be unavailable, blocked, or changed. The bot retries; check network access and the configured channel ID. |
| `{role_ping}` renders as nothing | `ROLE_ID` is not set. |
| The log shows `no data received for 180s` | The connection went silent and the bot reconnected on its own. If it happens constantly, run with `RUST_LOG=debug` and check your network. |

## Development

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

The same three checks run in CI on every push and pull request.

### Project layout

| Path | Purpose |
|---|---|
| `src/main.rs` | Startup, the connection loop, reconnect and backoff, and event handling |
| `src/kick.rs` | Pusher constants, message types and parsing |
| `src/config.rs` | Config types and loading |
| `src/discord.rs` | Placeholders, category rules, payload building and webhook delivery |
| `build.rs` | Embeds the icon in Windows builds |
| `config.example.json`, `.env.example` | Templates for local configuration |

## License

Released under the [MIT License](LICENSE).
