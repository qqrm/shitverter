use anyhow::{anyhow, Result as AnyResult};
use dotenvy::dotenv;
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
mod serverless;
mod telegram;

use handlers::process_video;
use limits::{utc_day_index, RateLimiter};

const DEFAULT_USER_DAILY_LIMIT: u32 = 10;
const DEFAULT_GLOBAL_DAILY_LIMIT: u32 = 50;
const DEFAULT_MAX_INPUT_BYTES: u32 = 100 * 1024 * 1024;
const DEFAULT_MAX_OUTPUT_BYTES: u32 = 49_000_000;
const DEFAULT_MAX_CONCURRENT_CONVERSIONS: u32 = 1;
const DEFAULT_MAX_CONVERSIONS_PER_RUN: u32 = 1;
const DEFAULT_MAX_UPDATES_PER_RUN: u32 = 100;

async fn ensure_bot_credentials(bot: &Bot) -> AnyResult<()> {
    bot.get_me()
        .send()
        .await
        .map_err(|_| anyhow!("Failed to authenticate bot with Telegram API"))?;
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

fn parse_telegram_user_id(value: &str) -> AnyResult<u64> {
    let user_id = value
        .parse::<u64>()
        .map_err(|_| anyhow!("ALLOWED_TELEGRAM_USER_ID must be a positive integer"))?;
    if user_id == 0 {
        return Err(anyhow!(
            "ALLOWED_TELEGRAM_USER_ID must be a positive integer"
        ));
    }
    Ok(user_id)
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
    let argument = std::env::args_os().nth(1);
    match argument.as_deref() {
        Some(argument) if argument == std::ffi::OsStr::new("--version") => {
            println!("converter-bot {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(argument) if argument == std::ffi::OsStr::new("--drain-queue") => {}
        Some(_) => return Err(anyhow!("usage: converter-bot [--version|--drain-queue]")),
        None => {}
    }

    dotenv().ok();
    pretty_env_logger::formatted_timed_builder()
        .parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();
    log::info!("Starting bot");

    let user_daily_limit = parse_env_limit("USER_DAILY_LIMIT", DEFAULT_USER_DAILY_LIMIT);
    let global_daily_limit = parse_env_limit("GLOBAL_DAILY_LIMIT", DEFAULT_GLOBAL_DAILY_LIMIT);
    let max_input_bytes = u64::from(parse_env_limit("MAX_INPUT_BYTES", DEFAULT_MAX_INPUT_BYTES));
    let max_output_bytes = u64::from(parse_env_limit(
        "MAX_OUTPUT_BYTES",
        DEFAULT_MAX_OUTPUT_BYTES,
    ));
    let max_concurrent_conversions = parse_env_limit(
        "MAX_CONCURRENT_CONVERSIONS",
        DEFAULT_MAX_CONCURRENT_CONVERSIONS,
    )
    .max(1) as usize;

    let bot = Bot::from_env();

    ensure_bot_credentials(&bot).await?;

    if argument.as_deref() == Some(std::ffi::OsStr::new("--drain-queue")) {
        let allowed_user_id = parse_telegram_user_id(
            &std::env::var("ALLOWED_TELEGRAM_USER_ID")
                .map_err(|_| anyhow!("ALLOWED_TELEGRAM_USER_ID is required in queue mode"))?,
        )?;
        let max_conversions_per_run =
            parse_env_limit("MAX_CONVERSIONS_PER_RUN", DEFAULT_MAX_CONVERSIONS_PER_RUN)
                .clamp(1, 20);
        let max_updates_per_run =
            parse_env_limit("MAX_UPDATES_PER_RUN", DEFAULT_MAX_UPDATES_PER_RUN).clamp(1, 100);
        return serverless::drain(
            &bot,
            max_input_bytes,
            max_output_bytes,
            max_conversions_per_run,
            max_updates_per_run,
            allowed_user_id,
        )
        .await;
    }

    let limiter = Arc::new(Mutex::new(RateLimiter::new(
        user_daily_limit,
        global_daily_limit,
    )));
    let monitor_limiter = Arc::clone(&limiter);
    let conversion_slots = Arc::new(tokio::sync::Semaphore::new(max_concurrent_conversions));

    {
        let limiter = limiter.lock().await;
        log::info!(
            "Limits initialized: day_index={}, user_daily_limit={}, global_daily_limit={}, max_input_bytes={}, max_output_bytes={}, max_concurrent_conversions={}, next_reset_in_seconds={}",
            limiter.current_day_index(),
            user_daily_limit,
            global_daily_limit,
            max_input_bytes,
            max_output_bytes,
            max_concurrent_conversions,
            next_midnight_utc_seconds(),
        );
    }

    tokio::spawn(async move {
        start_quota_monitor(monitor_limiter).await;
    });

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let limiter = Arc::clone(&limiter);
        let conversion_slots = Arc::clone(&conversion_slots);
        async move {
            if process_video(
                &bot,
                &msg,
                &limiter,
                &conversion_slots,
                max_input_bytes,
                max_output_bytes,
            )
            .await
            .is_err()
            {
                log::error!("Video processing failed");
            }
            respond(())
        }
    })
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_telegram_user_id;

    #[test]
    fn accepts_positive_telegram_user_id() {
        assert_eq!(parse_telegram_user_id("123456789").unwrap(), 123456789);
    }

    #[test]
    fn rejects_invalid_telegram_user_id() {
        assert!(parse_telegram_user_id("").is_err());
        assert!(parse_telegram_user_id("0").is_err());
        assert!(parse_telegram_user_id("-1").is_err());
        assert!(parse_telegram_user_id("not-an-id").is_err());
    }
}
