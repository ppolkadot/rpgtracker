# rpgtracker
dumb little bot to track cellbit lives - created for someone i like
all in.

It listens to Kick's realtime (Pusher) feed and posts a Discord webhook embed when a configured streamer goes live.

## Setup

1. Install Rust (edition 2024, so a recent stable toolchain).
2. Copy `.env.example` to `.env` (or `secrets.env`) and fill it in:

   | Variable | Required | Description |
   |---|---|---|
   | `DISCORD_WEBHOOK_URL` | yes | Webhook the alerts are posted to |
   | `ROLE_ID` | no | Discord role id used by `{role_ping}` (`PING_ROLE_ID` also works) |
   | `CONFIG_PATH` | no | Path to the streamer config, default `config.json` |
   | `RUST_LOG` | no | Log level, default `info` |

3. Copy `config.example.json` to `config.json` and edit it.
4. Run it:

   ```
   cargo run --release
   ```

`.env`, `secrets.env` and `config.json` are gitignored, so your webhook and settings stay local.

## Configuration (`config.json`)

Top level:

| Field | Description |
|---|---|
| `bot_name`, `bot_avatar` | Name and avatar shown on the Discord message |
| `cooldown_minutes` | Minimum minutes between alerts for one streamer (default `15`) |
| `streamers` | List of streamers to watch |

Per streamer:

| Field | Description |
|---|---|
| `username` | Kick username, used for the stream link |
| `channel_id` | Kick channel id |
| `ping_message` | Message text above the embed |
| `default_color` | Embed color as a decimal number |
| `custom_category` | Replaces the category name shown in `{category}` (or `null`) |
| `allowed_categories` | Only alert for these Kick categories (case-insensitive). Empty allows all, and so does an event that carries no category |
| `fields` | Extra embed fields: `name`, `value`, `inline` |
| `category_overrides` | Per Kick category: `label` and `color`. Wins over `custom_category` |

Placeholders usable in `ping_message` and in field names/values:
`{username}`, `{url}`, `{title}`, `{category}`, `{role_ping}`.

Notes:

- `config.json` is re-read on every live event, so most edits apply without a restart.
  Adding a new streamer is picked up on the next reconnect.
- If the config becomes invalid while running, the bot keeps using the last good one.
- Discord limits are enforced: content is cut to 2000 characters, titles to 256, at most 25 fields,
  and fields that render empty are dropped.
- The alert cooldown starts only after Discord accepted the message. Rate limits (429) and
  server errors are retried, for up to 3 attempts in total.
- Connection drops reconnect with exponential backoff (5s up to 5 minutes).

## Development

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```
