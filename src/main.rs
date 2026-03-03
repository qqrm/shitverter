use anyhow::{Context, Result as AnyResult};
use dotenv::dotenv;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use teloxide::prelude::*;
use tokio::{
    sync::Mutex,
    time::{sleep, Duration as TokioDuration},
};

// Модульная структура
mod converter;
mod handlers;
mod limits;
mod telegram;

use handlers::process_video;
use limits::{utc_day_index, RateLimiter};

const DEFAULT_USER_DAILY_LIMIT: u32 = 10;
const DEFAULT_GLOBAL_DAILY_LIMIT: u32 = 50;

async fn ensure_bot_credentials(bot: &Bot) -> AnyResult<()> {
    bot.get_me()
        .send()
        .await
        .context("Failed to authenticate bot with Telegram API. Check BOT_TOKEN")?;
    Ok(())
}

fn next_midnight_utc_seconds() -> u64 {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    let secs_into_day = now_secs % 86_400;
    if secs_into_day == 0 {
        86_400
    } else {
        86_400 - secs_into_day
    }
}

fn parse_env_limit(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

async fn start_quota_monitor(limiter: Arc<Mutex<RateLimiter>>) {
    loop {
        sleep(TokioDuration::from_secs(60)).await;
        let now_day_index = utc_day_index(SystemTime::now());
        let mut limiter = limiter.lock().await;
        if limiter.reset_if_new_day(now_day_index) {
            log::info!(
                "Daily quotas reset at UTC midnight: day_index={}, next_reset_in_seconds={}",
                now_day_index,
                next_midnight_utc_seconds(),
            );
        }
    }
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    dotenv().ok();
    pretty_env_logger::formatted_timed_builder()
        .parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();
    log::info!("Starting bot");

    let user_daily_limit = parse_env_limit("USER_DAILY_LIMIT", DEFAULT_USER_DAILY_LIMIT);
    let global_daily_limit = parse_env_limit("GLOBAL_DAILY_LIMIT", DEFAULT_GLOBAL_DAILY_LIMIT);

    let limiter = Arc::new(Mutex::new(RateLimiter::new(
        user_daily_limit,
        global_daily_limit,
    )));
    let monitor_limiter = Arc::clone(&limiter);

    {
        let limiter = limiter.lock().await;
        log::info!(
            "Rate limits initialized: day_index={}, user_daily_limit={}, global_daily_limit={}, next_reset_in_seconds={}",
            limiter.current_day_index(),
            user_daily_limit,
            global_daily_limit,
            next_midnight_utc_seconds(),
        );
    }

    tokio::spawn(async move {
        start_quota_monitor(monitor_limiter).await;
    });

    let bot = Bot::from_env();

    if let Err(error) = ensure_bot_credentials(&bot).await {
        log::error!("{error:?}");
        return Err(error);
    }

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let limiter = Arc::clone(&limiter);
        async move {
            if let Err(e) = process_video(&bot, &msg, &limiter).await {
                log::error!("Error processing video file: {:?}", e);
            }
            respond(())
        }
    })
    .await;
    Ok(())
}
