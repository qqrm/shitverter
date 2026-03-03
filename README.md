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

### Local Setup (alternative)
```bash
cargo build --release
./target/release/converter-bot
```

### Using Docker directly (alternative)
```bash
docker build -t shitverter .
docker run -d -e TELOXIDE_TOKEN=$TELEGRAM_API_TOKEN --name my_shitverter_container shitverter:latest
```

## Configuration
Set the following environment variables:
- `TELOXIDE_TOKEN`: Your Telegram Bot Token.

## Usage
Send a video file (for example, `.webm`, `.mkv`, `.mov`) to the chat with the bot, and it will send a converted `.mp4` file and delete the original message.
Also shows tg ID's of new members.

## Rate limits
- `USER_DAILY_LIMIT` (default: `10`) — maximum conversions per user per UTC day.
- `GLOBAL_DAILY_LIMIT` (default: `50`) — maximum conversions for the whole bot per UTC day.

The bot logs quota decisions and resets counters at UTC midnight.

## Contributing
Contributions are welcome. Please send pull requests.
