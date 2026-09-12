use anyhow::{anyhow, Context, Result};
use teloxide::prelude::*;
use teloxide::types::{AllowedUpdate, Message, ReplyParameters, Update, UpdateKind};
use tokio::sync::{Mutex, Semaphore};

use crate::handlers::{process_video, ProcessOutcome};
use crate::limits::RateLimiter;

pub async fn drain(
    bot: &Bot,
    max_input_bytes: u64,
    max_output_bytes: u64,
    max_media_jobs: u32,
    max_updates: u32,
    allowed_user_id: u64,
) -> Result<()> {
    let limiter = Mutex::new(RateLimiter::new(u32::MAX, u32::MAX));
    let conversion_slot = Semaphore::new(1);
    let mut acknowledged = 0_u32;
    let mut media_jobs = 0_u32;
    let mut successful_conversions = 0_u32;
    let mut offset = None;

    while acknowledged < max_updates && media_jobs < max_media_jobs {
        let Some(update) = next_update(bot, offset).await? else {
            break;
        };
        let next_offset = update_offset(&update)?;

        let outcome = match message_from_update(update) {
            Some(message) if sender_is_allowed(&message, allowed_user_id) => {
                match process_video(
                    bot,
                    &message,
                    &limiter,
                    &conversion_slot,
                    max_input_bytes,
                    max_output_bytes,
                )
                .await
                {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        log::warn!("Media processing failed; sending a generic failure response");
                        notify_failure(bot, &message).await?;
                        ProcessOutcome::HandledWithoutConversion
                    }
                }
            }
            Some(_) => {
                log::warn!("Discarded an update from a sender outside the allowlist");
                ProcessOutcome::Ignored
            }
            None => ProcessOutcome::Ignored,
        };

        offset = Some(next_offset);
        acknowledged += 1;
        if outcome != ProcessOutcome::Ignored {
            media_jobs += 1;
        }
        if outcome == ProcessOutcome::Converted {
            successful_conversions += 1;
        }
    }

    if let Some(offset) = offset {
        acknowledge_through(bot, offset).await?;
    }

    log::info!(
        "Queue drain completed: acknowledged_updates={}, media_jobs={}, successful_conversions={}",
        acknowledged,
        media_jobs,
        successful_conversions
    );
    Ok(())
}

fn sender_is_allowed(message: &Message, allowed_user_id: u64) -> bool {
    message
        .from
        .as_ref()
        .map(|user| user.id.0 == allowed_user_id)
        .unwrap_or(false)
}

async fn next_update(bot: &Bot, offset: Option<i32>) -> Result<Option<Update>> {
    let request = bot
        .get_updates()
        .limit(1)
        .timeout(0)
        .allowed_updates([AllowedUpdate::Message, AllowedUpdate::ChannelPost]);
    let updates = match offset {
        Some(offset) => request.offset(offset).send().await,
        None => request.send().await,
    }
    .map_err(|_| anyhow!("Telegram queue polling failed"))?;

    Ok(updates.into_iter().next())
}

async fn acknowledge_through(bot: &Bot, offset: i32) -> Result<()> {
    bot.get_updates()
        .offset(offset)
        .limit(1)
        .timeout(0)
        .allowed_updates([AllowedUpdate::Message, AllowedUpdate::ChannelPost])
        .send()
        .await
        .map_err(|_| anyhow!("Telegram queue acknowledgement failed"))?;
    Ok(())
}

fn update_offset(update: &Update) -> Result<i32> {
    let id = i32::try_from(update.id.0).context("Telegram update id exceeds i32")?;
    id.checked_add(1).context("Telegram update id overflow")
}

fn message_from_update(update: Update) -> Option<Message> {
    match update.kind {
        UpdateKind::Message(message) | UpdateKind::ChannelPost(message) => Some(message),
        _ => None,
    }
}

async fn notify_failure(bot: &Bot, message: &Message) -> Result<()> {
    let mut request = bot
        .send_message(
            message.chat.id,
            "Не удалось сконвертировать файл. Задание удалено из очереди.",
        )
        .reply_parameters(ReplyParameters::new(message.id).allow_sending_without_reply());
    if let Some(thread_id) = message.thread_id {
        request = request.message_thread_id(thread_id);
    }
    request
        .await
        .map_err(|_| anyhow!("failed to send Telegram failure response"))?;
    Ok(())
}
