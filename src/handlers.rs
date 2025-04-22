use anyhow::{Context, Result as AnyResult};
use teloxide::{
    prelude::*,
    types::{InputFile, MediaKind, MessageKind, ParseMode},
};
use tokio::{fs, task};

use crate::telegram::download_file;
use crate::converter::convert_webm_to_mp4;

pub async fn process_webm(bot: &Bot, msg: &Message) -> AnyResult<()> {
    let MessageKind::Common(common) = &msg.kind else {
        return Ok(());
    };
    let MediaKind::Document(document) = &common.media_kind else {
        return Ok(());
    };
    let Some(mime_type) = &document.document.mime_type else {
        return Ok(());
    };

    if mime_type.essence_str() != "video/webm" {
        return Ok(());
    };

    // Скачиваем файл.
    let file_path = download_file(bot, &document.document.file.id).await?;

    // Клонируем file_path для передачи в замыкание, чтобы оригинал оставался доступен
    let file_path_clone = file_path.clone();

    // Конвертация файла выполняется в отдельном блокирующем потоке.
    let join_result = task::spawn_blocking(move || convert_webm_to_mp4(&file_path_clone))
        .await
        .context("Failed to join blocking task")?;
    let converted_file_path = join_result.context("FFmpeg conversion failed")?;
    
    // Формируем запрос на отправку видео.
    let mut send_video_request = bot
        .send_video(msg.chat.id, InputFile::file(&converted_file_path))
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

    // Асинхронное удаление временных файлов с логированием ошибок.
    if let Err(e) = fs::remove_file(&file_path).await {
        log::error!("Error deleting file {}: {:?}", file_path, e);
    }
    if let Err(e) = fs::remove_file(&converted_file_path).await {
        log::error!("Error deleting file {}: {:?}", converted_file_path, e);
    }

    Ok(())
} 