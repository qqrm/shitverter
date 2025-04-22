use anyhow::Result as AnyResult;
use dotenv::dotenv;
use teloxide::prelude::*;

// Модульная структура
mod telegram;
mod converter;
mod handlers;

use handlers::process_webm;

#[tokio::main]
async fn main() -> AnyResult<()> {
    dotenv().ok();
    pretty_env_logger::init();
    log::info!("Starting bot");

    let bot = Bot::from_env();

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        if let Err(e) = process_webm(&bot, &msg).await {
            log::error!("Error processing webm file: {:?}", e);
        }
        respond(())
    })
    .await;
    Ok(())
} 