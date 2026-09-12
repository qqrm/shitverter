use anyhow::{Context, Result as AnyResult};
use teloxide::{
    prelude::*,
    types::{InputFile, MediaKind, MessageKind, ParseMode, ReplyParameters},
};
use tokio::{
    fs,
    sync::{Mutex, Semaphore},
    task,
};

use crate::converter::convert_video_to_mp4;
use crate::limits::{utc_day_index, QuotaDecision, RateLimiter};
use crate::telegram::{download_file, FileTooLargeError};

const VIDEO_FILE_EXTENSIONS: &[&str] = &[
    "3gp", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "webm", "wmv",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessOutcome {
    Ignored,
    HandledWithoutConversion,
    Converted,
}

fn has_video_extension(file_name: &str) -> bool {
    file_name
        .rsplit_once('.')
        .map(|(_, ext)| VIDEO_FILE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn is_video_document(mime_type: Option<&str>, file_name: Option<&str>) -> bool {
    if mime_type
        .map(|mime| mime.starts_with("video/"))
        .unwrap_or(false)
    {
        return true;
    }

    file_name.map(has_video_extension).unwrap_or(false)
}

fn quota_subject_key(msg: &Message) -> i64 {
    if let Some(user) = msg.from.as_ref() {
        return user.id.0 as i64;
    }

    // For channel/anonymous messages without `from`, avoid a shared bucket (0)
    // by assigning a synthetic per-message key scoped by chat + message id.
    synthetic_quota_key(msg.chat.id.0, i64::from(msg.id.0))
}

fn synthetic_quota_key(chat_id: i64, message_id: i64) -> i64 {
    chat_id.wrapping_mul(1_000_003).wrapping_add(message_id) ^ i64::MIN
}

fn escape_markdown_v2(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(
            character,
            '_' | '*'
                | '['
                | ']'
                | '('
                | ')'
                | '~'
                | '`'
                | '>'
                | '#'
                | '+'
                | '-'
                | '='
                | '|'
                | '{'
                | '}'
                | '.'
                | '!'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn file_too_large_message(error: &FileTooLargeError) -> String {
    match error.limit_bytes {
        Some(limit_bytes) => format!(
            "Файл слишком большой. Максимальный размер для обработки — {} МиБ.",
            limit_bytes / (1024 * 1024),
        ),
        None => "Файл слишком большой: Telegram не позволяет боту его скачать. Пришли файл меньшего размера."
            .to_string(),
    }
}

pub async fn process_video(
    bot: &Bot,
    msg: &Message,
    limiter: &Mutex<RateLimiter>,
    conversion_slots: &Semaphore,
    max_input_bytes: u64,
    max_output_bytes: u64,
) -> AnyResult<ProcessOutcome> {
    let user_id = quota_subject_key(msg);
    log::info!("Received a Telegram message");

    let MessageKind::Common(common) = &msg.kind else {
        return Ok(ProcessOutcome::Ignored);
    };

    let file_id = match &common.media_kind {
        MediaKind::Video(video) => {
            log::info!("Accepted a video message for validation");
            video.video.file.id.to_string()
        }
        MediaKind::Document(document) => {
            let mime_type = document
                .document
                .mime_type
                .as_ref()
                .map(|mime| mime.essence_str());
            let file_name = document.document.file_name.as_deref();

            if !is_video_document(mime_type, file_name) {
                return Ok(ProcessOutcome::Ignored);
            }

            log::info!("Accepted a video document for validation");

            document.document.file.id.to_string()
        }
        _ => return Ok(ProcessOutcome::Ignored),
    };

    let quota_decision = {
        let mut limiter = limiter.lock().await;
        limiter.check_and_consume(user_id, utc_day_index(std::time::SystemTime::now()))
    };

    let consumed_day_index = match quota_decision {
        QuotaDecision::Allowed {
            user_count,
            user_limit,
            global_count,
            global_limit,
            day_index,
        } => {
            log::info!(
                "Quota allowed: day={}, user_count={}/{}, global_count={}/{}",
                day_index,
                user_count,
                user_limit,
                global_count,
                global_limit,
            );
            day_index
        }
        QuotaDecision::UserLimitExceeded {
            user_count,
            user_limit,
            global_count,
            global_limit,
            day_index,
        } => {
            log::warn!(
                "User daily limit exceeded: day={}, user_count={}/{}, global_count={}/{}",
                day_index,
                user_count,
                user_limit,
                global_count,
                global_limit,
            );
            bot.send_message(
                msg.chat.id,
                format!(
                    "Daily limit exceeded: {}/{} videos for today. Try again tomorrow (UTC).",
                    user_count, user_limit
                ),
            )
            .await?;
            return Ok(ProcessOutcome::HandledWithoutConversion);
        }
        QuotaDecision::GlobalLimitExceeded {
            global_count,
            global_limit,
            day_index,
        } => {
            log::warn!(
                "Global daily limit exceeded: day={}, global_count={}/{}",
                day_index,
                global_count,
                global_limit,
            );
            bot.send_message(
                msg.chat.id,
                "Service daily conversion limit is exhausted. Please try again tomorrow (UTC).",
            )
            .await?;
            return Ok(ProcessOutcome::HandledWithoutConversion);
        }
    };

    let _conversion_permit = conversion_slots
        .acquire()
        .await
        .context("Conversion queue is unavailable")?;

    // Скачиваем файл.
    let file_path = match download_file(bot, &file_id, max_input_bytes).await {
        Ok(file_path) => file_path,
        Err(error) => {
            let Some(file_too_large) = error.downcast_ref::<FileTooLargeError>() else {
                return Err(error);
            };

            let refunded = limiter.lock().await.refund(user_id, consumed_day_index);
            log::warn!(
                "Rejected oversized file: quota_refunded={}, reason={}",
                refunded,
                file_too_large,
            );

            let mut request = bot
                .send_message(msg.chat.id, file_too_large_message(file_too_large))
                .reply_parameters(ReplyParameters::new(msg.id).allow_sending_without_reply());
            if let Some(thread_id) = msg.thread_id {
                request = request.message_thread_id(thread_id);
            }
            request.await?;
            return Ok(ProcessOutcome::HandledWithoutConversion);
        }
    };

    let mut converted_file_path: Option<String> = None;

    let processing_result: AnyResult<ProcessOutcome> = async {
        // Клонируем file_path для передачи в замыкание, чтобы оригинал оставался доступен
        let file_path_clone = file_path.clone();

        // Конвертация файла выполняется в отдельном блокирующем потоке.
        let join_result = task::spawn_blocking(move || convert_video_to_mp4(&file_path_clone))
            .await
            .context("Failed to join blocking task")?;
        let converted_path = join_result.context("FFmpeg conversion failed")?;
        converted_file_path = Some(converted_path.clone());

        if fs::metadata(&converted_path).await?.len() > max_output_bytes {
            bot.send_message(
                msg.chat.id,
                "После конвертации файл получился слишком большим для отправки через Telegram Bot API.",
            )
            .await?;
            return Ok(ProcessOutcome::HandledWithoutConversion);
        }

        // Формируем запрос на отправку видео.
        let mut send_video_request = bot
            .send_video(msg.chat.id, InputFile::file(&converted_path))
            .disable_notification(true);

        if let Some(thread_id) = msg.thread_id {
            send_video_request = send_video_request.message_thread_id(thread_id);
        }

        if let Some(user) = msg.from.as_ref() {
            let full_name = escape_markdown_v2(&user.full_name());
            let signature = format!("send by [{}](tg://user?id={})", full_name, user.id);
            let caption = msg.caption().map_or_else(
                || signature.clone(),
                |existing_caption| {
                    format!("{}\n\n{}", escape_markdown_v2(existing_caption), signature)
                },
            );
            send_video_request = send_video_request.caption(caption);
        }

        if let Some(reply_msg) = msg.reply_to_message() {
            send_video_request = send_video_request
                .reply_parameters(ReplyParameters::new(reply_msg.id).allow_sending_without_reply());
        }

        send_video_request = send_video_request.parse_mode(ParseMode::MarkdownV2);
        send_video_request.await?;

        // Удаляем оригинальное сообщение.
        if bot.delete_message(msg.chat.id, msg.id).await.is_err() {
            log::warn!("Converted video was sent, but the source message could not be deleted");
        }

        Ok(ProcessOutcome::Converted)
    }
    .await;

    if let Err(e) = fs::remove_file(&file_path).await {
        log::error!("Failed to delete a temporary input file: {e}");
    }
    if let Some(converted_path) = converted_file_path {
        if let Err(e) = fs::remove_file(&converted_path).await {
            log::error!("Failed to delete a temporary output file: {e}");
        }
    }

    processing_result
}

#[cfg(test)]
mod tests {
    use super::{
        escape_markdown_v2, file_too_large_message, is_video_document, synthetic_quota_key,
    };
    use crate::telegram::FileTooLargeError;

    #[test]
    fn detects_video_mime_type() {
        assert!(is_video_document(
            Some("video/x-matroska"),
            Some("source.bin")
        ));
    }

    #[test]
    fn detects_video_extension_without_video_mime() {
        assert!(is_video_document(
            Some("application/octet-stream"),
            Some("source.MKV"),
        ));
    }

    #[test]
    fn skips_non_video_documents() {
        assert!(!is_video_document(
            Some("application/pdf"),
            Some("document.pdf")
        ));
    }

    #[test]
    fn generates_non_zero_synthetic_quota_key() {
        assert_ne!(synthetic_quota_key(-1001234567890, 42), 0);
        assert_ne!(
            synthetic_quota_key(-1001234567890, 42),
            synthetic_quota_key(-1001234567890, 43)
        );
    }

    #[test]
    fn escapes_untrusted_markdown_v2_caption_text() {
        assert_eq!(escape_markdown_v2("name_[x]!"), r"name\_\[x\]\!");
    }

    #[test]
    fn explains_configured_file_size_limit() {
        assert_eq!(
            file_too_large_message(&FileTooLargeError {
                limit_bytes: Some(100 * 1024 * 1024),
            }),
            "Файл слишком большой. Максимальный размер для обработки — 100 МиБ.",
        );
    }

    #[test]
    fn explains_telegram_download_limit() {
        assert_eq!(
            file_too_large_message(&FileTooLargeError { limit_bytes: None }),
            "Файл слишком большой: Telegram не позволяет боту его скачать. Пришли файл меньшего размера.",
        );
    }
}
