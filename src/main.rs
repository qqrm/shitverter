use anyhow::{Context, Result as AnyResult};
use dotenv::dotenv;
use teloxide::prelude::*;

// Модульная структура
mod converter;
mod handlers;
mod telegram;

use handlers::process_video;

async fn ensure_bot_credentials(bot: &Bot) -> AnyResult<()> {
    bot.get_me()
        .send()
        .await
        .context("Failed to authenticate bot with Telegram API. Check BOT_TOKEN")?;
    Ok(())
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    dotenv().ok();
    pretty_env_logger::init();
    log::info!("Starting bot");

    let bot = Bot::from_env();

    if let Err(error) = ensure_bot_credentials(&bot).await {
        log::error!("{error:?}");
        return Err(error);
    }

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        if let Err(e) = process_video(&bot, &msg).await {
            log::error!("Error processing video file: {:?}", e);
        }
        respond(())
    })
    .await;
    Ok(())
}
