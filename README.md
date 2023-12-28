# Shitverer Bot

## Introduction
This bot finds and processes `.webm` files in chats and converts them to `.mp4`.

## Requirements
- Rust
- FFmpeg
- Docker (optional)

## Installation and Running
### Local Setup
cargo build --release
./target/release/my_telegram_bot

### Using Docker
docker build -t my_telegram_bot .
docker run my_telegram_bot

## Configuration
Set the following environment variables:
- `TELEGRAM_BOT_TOKEN`: Your Telegram Bot Token.

## Usage
Send a `.webm` file to the bot, and it will reply with a converted `.mp4` file.

## Contributing
Contributions are welcome. Please send pull requests.

## License
[Your chosen license]