# Shitverter Bot

## Introduction
This bot waits and processes `.webm` files in chats and converts them to `.mp4`.

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
Send a `.webm` file to the chat with bot, and it will send converted `.mp4` file and delete post with webm.
Also shows tg ID's of new members.

## Contributing
Contributions are welcome. Please send pull requests.
