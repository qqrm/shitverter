use anyhow::{Context, Result as AnyResult};
use teloxide::{
    prelude::*,
    types::{InputFile, MediaKind, MessageKind, ParseMode},
};
use tokio::{fs, sync::Mutex, task};

use crate::converter::convert_video_to_mp4;
use crate::limits::{utc_day_index, QuotaDecision, RateLimiter};
use crate::telegram::download_file;

const VIDEO_FILE_EXTENSIONS: &[&str] = &[
    "3gp", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "webm", "wmv",
];

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

fn sanitize_user_name(name: Option<&str>) -> String {
    let Some(name) = name else {
        return "unknown".to_string();
    };

    if name.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || ch.is_ascii_whitespace()
            || ('А'..='я').contains(&ch)
            || matches!(ch, 'Ё' | 'ё' | '_' | '-' | '.' | ' ')
    }) {
        name.to_string()
    } else {
        "eblan with symbols".to_string()
    }
}

pub async fn process_video(
    bot: &Bot,
    msg: &Message,
    limiter: &Mutex<RateLimiter>,
) -> AnyResult<()> {
    let user = msg.from();
    let user_id = user.map_or(0, |u| u.id.0 as i64);
    let user_name = sanitize_user_name(user.map(|u| u.full_name()).as_deref());

    log::info!(
        "Incoming message: chat_id={}, message_id={}, user_id={}, user_name='{}'",
        msg.chat.id,
        msg.id,
        user_id,
        user_name,
    );

    let MessageKind::Common(common) = &msg.kind else {
        return Ok(());
    };

    let file_id = match &common.media_kind {
        MediaKind::Video(video) => {
            log::info!(
                "Incoming video message: chat_id={}, message_id={}, mime={:?}, file_name={:?}, file_id={}",
                msg.chat.id,
                msg.id,
                video.video.mime_type,
                video.video.file_name,
                video.video.file.id
            );
            video.video.file.id.to_string()
        }
        MediaKind::Document(document) => {
            let mime_type = document
                .document
                .mime_type
                .as_ref()
                .map(|mime| mime.essence_str());
            let file_name = document.document.file_name.as_deref();

            log::info!(
                "Incoming document message: chat_id={}, message_id={}, mime={:?}, file_name={:?}, file_id={}",
                msg.chat.id,
                msg.id,
                mime_type,
                file_name,
                document.document.file.id
            );

            if !is_video_document(mime_type, file_name) {
                return Ok(());
            }

            document.document.file.id.to_string()
        }
        _ => return Ok(()),
    };

    let quota_decision = {
        let mut limiter = limiter.lock().await;
        limiter.check_and_consume(user_id, utc_day_index(std::time::SystemTime::now()))
    };

    match quota_decision {
        QuotaDecision::Allowed {
            user_count,
            user_limit,
            global_count,
            global_limit,
            day_index,
        } => {
            log::info!(
                "Quota allowed: day={}, user_id={}, user_count={}/{}, global_count={}/{}",
                day_index,
                user_id,
                user_count,
                user_limit,
                global_count,
                global_limit,
            );
        }
        QuotaDecision::UserLimitExceeded {
            user_count,
            user_limit,
            global_count,
            global_limit,
            day_index,
        } => {
            log::warn!(
                "User daily limit exceeded: day={}, user_id={}, user_count={}/{}, global_count={}/{}",
                day_index,
                user_id,
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
            return Ok(());
        }
        QuotaDecision::GlobalLimitExceeded {
            global_count,
            global_limit,
            day_index,
        } => {
            log::warn!(
                "Global daily limit exceeded: day={}, user_id={}, global_count={}/{}",
                day_index,
                user_id,
                global_count,
                global_limit,
            );
            bot.send_message(
                msg.chat.id,
                "Service daily conversion limit is exhausted. Please try again tomorrow (UTC).",
            )
            .await?;
            return Ok(());
        }
    }

    // Скачиваем файл.
    let file_path = download_file(bot, &file_id).await?;

    let mut converted_file_path: Option<String> = None;

    let processing_result: AnyResult<()> = async {
        // Клонируем file_path для передачи в замыкание, чтобы оригинал оставался доступен
        let file_path_clone = file_path.clone();

        // Конвертация файла выполняется в отдельном блокирующем потоке.
        let join_result = task::spawn_blocking(move || convert_video_to_mp4(&file_path_clone))
            .await
            .context("Failed to join blocking task")?;
        let converted_path = join_result.context("FFmpeg conversion failed")?;
        converted_file_path = Some(converted_path.clone());

        // Формируем запрос на отправку видео.
        let mut send_video_request = bot
            .send_video(msg.chat.id, InputFile::file(&converted_path))
            .disable_notification(true);

        if let Some(thread_id) = msg.thread_id {
            send_video_request = send_video_request.message_thread_id(thread_id);
        }

        if let Some(user) = msg.from() {
            let full_name = user.full_name();
            let signature = format!("send by [{}](tg://user?id={})", full_name, user.id);
            let caption = msg.caption().map_or_else(
                || signature.clone(),
                |existing_caption| format!("{}\n\n{}", existing_caption, signature),
            );
            send_video_request = send_video_request
                .caption(caption)
                .allow_sending_without_reply(true);
        }

        if let Some(reply_msg) = msg.reply_to_message() {
            send_video_request = send_video_request.reply_to_message_id(reply_msg.id);
        }

        send_video_request = send_video_request.parse_mode(ParseMode::MarkdownV2);
        send_video_request.await?;

        // Удаляем оригинальное сообщение.
        bot.delete_message(msg.chat.id, msg.id).await?;

        Ok(())
    }
    .await;

    if let Err(e) = fs::remove_file(&file_path).await {
        log::error!("Error deleting file {}: {:?}", file_path, e);
    }
    if let Some(converted_path) = converted_file_path {
        if let Err(e) = fs::remove_file(&converted_path).await {
            log::error!("Error deleting file {}: {:?}", converted_path, e);
        }
    }

    processing_result
}

#[cfg(test)]
mod tests {
    use super::{is_video_document, sanitize_user_name};

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
    fn keeps_safe_name() {
        assert_eq!(sanitize_user_name(Some("Иван Ivan_01")), "Иван Ivan_01");
    }

    #[test]
    fn replaces_unsafe_name() {
        assert_eq!(sanitize_user_name(Some("bad🚀name")), "eblan with symbols");
    }
}
