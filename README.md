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
docker build -t shitverter .
docker run -d --env-file .env --name my_shitverter_container shitverter:latest
```

## Configuration
Set the following environment variables:
- `TELOXIDE_TOKEN`: Your Telegram Bot Token.
- `RUST_LOG`: Log level filter for the bot output (example: `RUST_LOG=info`, default: `info`).

If a local `.env` file exists, `run.sh` and `rebuild.sh` automatically pass it to
`docker run` using `--env-file .env`.

## Usage
Send a video file (for example, `.webm`, `.mkv`, `.mov`) to the chat with the bot, and it will send a converted `.mp4` file and delete the original message.
Also shows tg ID's of new members.

## Rate limits
- `USER_DAILY_LIMIT` (default: `10`) — maximum conversions per user per UTC day.
- `GLOBAL_DAILY_LIMIT` (default: `50`) — maximum conversions for the whole bot per UTC day.

The bot logs quota decisions and resets counters at UTC midnight.

## Contributing
Contributions are welcome. Please send pull requests.
