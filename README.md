# Shitverter Bot

## Introduction
This bot waits for video files in chats and converts them to `.mp4` using FFmpeg.

## Requirements
- Rust
- FFmpeg
- Docker (optional)
- [Just](https://github.com/casey/just) (command runner)

## Installation and Running

### Using Just (recommended)
```bash
# Show available commands
just

# Rebuild the project from scratch and run in docker
just rebuild

# Just run the container (without rebuilding)
just run
```

### Using helper scripts

```bash
# Rebuild image + run container
./rebuild.sh

# Run existing image
./run.sh
```

### Local Setup (alternative)

```bash
cargo build --release
./target/release/converter-bot
```

### Using Docker directly (alternative)

```bash
# Build production image (multi-stage Dockerfile; final stage is the runtime image).
docker build -t shitverter:latest .

# Run it.
docker run -d --env-file .env --name my_shitverter_container shitverter:latest

# Optional: build the reusable toolchain image (Rust + musl) for local builds/debugging.
docker build --target build-env -t shitverter-buildenv:latest .

# Optional: pin the Rust toolchain image for reproducible builds.
docker build --build-arg MUSLRUST_TAG=1.93.1-stable-2026-02-23 -t shitverter:latest .
```

## Configuration

Set the following environment variables:

* `TELOXIDE_TOKEN`: Your Telegram Bot Token.
* `TELOXIDE_API_URL` (default: `https://api.telegram.org`) — Bot API base URL.
* `TELEGRAM_LOCAL_FILES_DIR` (optional) — shared absolute cache directory for a
  local Bot API server started with `--local`. Requires a non-cloud
  `TELOXIDE_API_URL`; the directory must exist and be readable by the bot.
* `TELEGRAM_REQUEST_TIMEOUT_SECONDS` (default: `300`) — HTTP request timeout.
  Local `getFile` downloads the media before responding, so allow enough time
  for the largest accepted input and output uploads.
* `ALLOWED_TELEGRAM_USER_ID`: required in `--drain-queue` mode; only updates
  whose sender has this numeric Telegram user ID may reach the download and
  conversion path. All other updates are acknowledged and silently discarded.
* `RUST_LOG`: Log level filter for the bot output (example: `RUST_LOG=info`, default: `info`).
* `USER_DAILY_LIMIT` (default: `10`) — maximum conversions per user per UTC day.
* `GLOBAL_DAILY_LIMIT` (default: `50`) — maximum conversions for the whole bot per UTC day.
* `MAX_INPUT_BYTES` (default: `104857600`, 100 MiB) — maximum downloaded source-video size.
* `MAX_OUTPUT_BYTES` (default: `49000000`) — maximum converted-video size before upload.
* `MAX_CONCURRENT_CONVERSIONS` (default: `1`) — simultaneous download-and-FFmpeg jobs.

## Files larger than 20 MB

Telegram can accept a video in a chat while its cloud Bot API refuses to let a
bot download it: cloud `getFile` is limited to **20 MB**. Raising
`MAX_INPUT_BYTES` alone cannot change that transport limit.

For the Docker deployment, add `TELEGRAM_API_ID` and `TELEGRAM_API_HASH` to the
GitHub `production` environment secrets and run **Deploy** with
`enable_local_api=true`. It builds and starts the official API container,
performs the session migration, and configures inputs up to **2 GiB** and outputs
up to **2000 MB**. See [activation and recovery](deploy/README.md#enable-large-files-from-github-actions).

Run the official [Telegram Bot API server](https://github.com/tdlib/telegram-bot-api)
with `--local`, then configure Shitverter to use it:

```dotenv
TELOXIDE_API_URL=http://telegram-bot-api:8081
TELEGRAM_LOCAL_FILES_DIR=/var/lib/telegram-bot-api
TELEGRAM_REQUEST_TIMEOUT_SECONDS=1800
MAX_INPUT_BYTES=2147483648
MAX_OUTPUT_BYTES=2000000000
```

Mount the Bot API data directory at the **same absolute path** in both containers;
the bot only needs read access. Local `getFile` returns an absolute path, and
Shitverter streams a bounded copy into its private conversion workspace. It
rejects paths outside the configured directory (including symlink escapes),
keeps the API server's original cached file, and removes temporary copies after
processing. The API endpoint alone does not provide an HTTP file server.

The local API removes the cloud download limit and supports uploads up to
**2000 MB**, but the bot's own resource limits remain independent: the defaults
still accept at most **100 MiB** input and **49,000,000 bytes** output. Increase
both limits deliberately when needed, and provision storage for the API cache,
the input copy, and the converted output. The advertised input size is checked
before `getFile`; returned metadata and the actual bytes copied are checked too.

The server requires your Telegram `api_id` and `api_hash`. To migrate an existing
bot, stop its old update consumer and call the cloud API's `logOut` before
starting it against the local endpoint. Telegram prevents returning to the cloud
API for 10 minutes after `logOut`. Both long-running and `--drain-queue` modes use
the same configuration; an ephemeral worker still needs access to the shared
directory. See [deployment instructions](deploy/README.md) for the container
setup and [Telegram's local API documentation](https://core.telegram.org/bots/api#using-a-local-bot-api-server)
for the transport limits and migration requirements.

## One-shot queue mode

Released binaries also support a bounded, stateless queue-drain mode for ephemeral
CI workers:

```bash
converter-bot --drain-queue
```

It consumes Telegram `getUpdates` sequentially, handles at most one media job and
100 updates by default, then exits. Override those bounds with
`MAX_CONVERSIONS_PER_RUN` and `MAX_UPDATES_PER_RUN`. Do not run this mode while the
long-running bot or a webhook is consuming updates for the same token.

`ALLOWED_TELEGRAM_USER_ID` is mandatory in queue mode. Authorization is checked
before Telegram file metadata is requested or media is downloaded. Messages
without a sender user (including channel posts) are rejected.

Queue delivery is at-least-once: a crash after sending a result but before
acknowledging its update can create a duplicate. Pending Telegram updates are not
a durable replacement for a database-backed queue.

If a local `.env` file exists, `run.sh` and `rebuild.sh` automatically pass it to
`docker run` using `--env-file .env`.

## Usage

Send a video file (for example, `.webm`, `.mkv`, `.mov`) to the chat with the bot, and it will send a converted `.mp4` file and delete the original message.
Also shows tg ID's of new members.

## Rate limits

The bot logs quota decisions and resets counters at UTC midnight. Limits are held only in memory, so a restart resets them.

## Contributing

Contributions are welcome. Please send pull requests.

The checked-in toolchain follows Rust's `stable` channel. Fresh CI workers
therefore pick up each stable Rust release. Dependabot checks Cargo, Docker, and
GitHub Actions dependencies weekly; patch and minor updates merge automatically
only after every reported check succeeds. Major updates remain open for review.

## CI and production deployment

GitHub Actions validates formatting, tests, lints, RustSec advisories, and the
production Docker build for pull requests and `main`. A separate manually
promoted GitHub Actions workflow can publish an immutable GHCR image and deploy
it to the bot VM without copying the Telegram token to GitHub. See
[`deploy/README.md`](deploy/README.md) for VM bootstrap and the required GitHub
environment secrets.
