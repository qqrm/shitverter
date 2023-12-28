use dotenv::dotenv;
use std::env;
use teloxide::prelude::*;

#[tokio::main]
async fn main() {
    // Initialize environment variables and logger once
    dotenv().ok();
    pretty_env_logger::init();
    log::info!("Starting converter-bot");

    // Get the bot token from environment variables
    let bot_token = env::var("BOT_TOKEN").expect("BOT_TOKEN not set");
    let bot = teloxide::Bot::new(bot_token);

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        bot.send_dice(msg.chat.id).await?;
        Ok(())
    })
    .await;
}
