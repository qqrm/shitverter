# Shitverer Bot

## Introduction
This bot waits and processes `.webm` files in chats and converts them to `.mp4`.

## Requirements
- Rust
- FFmpeg
- Docker (optional)

## Installation and Running
### Local Setup
cargo build --release
./target/release/converter-bot

### Using Docker
docker build -t converter-bot .
docker run converter-bot

## Configuration
Set the following environment variables:
- `TELEGRAM_BOT_TOKEN`: Your Telegram Bot Token.

## Usage
Send a `.webm` file to the chat with bot, and it will send converted `.mp4` file and delete post with webm.

## Contributing
Contributions are welcome. Please send pull requests.