use anyhow::{anyhow, Context, Result as AnyResult};
use dotenv::dotenv;
use std::process::Command;
use teloxide::{
    prelude::*,
    types::{InputFile, MediaKind, MessageKind, ParseMode},
};
use tokio::fs;
use tokio::task;

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
        if let Err(e) = process_new_member(&bot, &msg).await {
            log::error!("Error processing new member: {:?}", e);
        }
        respond(())
    })
    .await;
    Ok(())
}

/// Обрабатывает присоединение нового участника и отправляет сообщение с его данными.
async fn process_new_member(bot: &Bot, msg: &Message) -> AnyResult<()> {
    if let MessageKind::NewChatMembers(new_members_msg) = &msg.kind {
        let resp_with_ids: String = new_members_msg
            .new_chat_members
            .iter()
            .map(|user| {
                format!(
                    "Check ASAP [{}](tg://user?id={}) with id {}\n",
                    user.full_name(),
                    user.id,
                    user.id
                )
            })
            .collect();

        bot.send_message(msg.chat.id, resp_with_ids)
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
    }
    Ok(())
}

async fn process_webm(bot: &Bot, msg: &Message) -> AnyResult<()> {
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

/// Скачивает файл с серверов Telegram по его идентификатору.
async fn download_file(bot: &Bot, file_id: &str) -> AnyResult<String> {
    let file = bot.get_file(file_id).send().await?;
    let download_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot.token(),
        file.path
    );
    let response = reqwest::get(&download_url).await?;
    let file_path = format!("/tmp/{}.webm", file_id);
    let content = response.bytes().await?;
    fs::write(&file_path, &content).await?;
    Ok(file_path)
}

/// Конвертирует файл `.webm` в формат `.mp4` с помощью FFmpeg.
fn convert_webm_to_mp4(file_path: &str) -> AnyResult<String> {
    let output_path = format!("{}.mp4", file_path);
    let output = Command::new("ffmpeg")
        .args(["-i", file_path, &output_path])
        .output()?;
    if !output.status.success() {
        return Err(anyhow!("FFmpeg conversion failed: {:?}", output));
    }
    Ok(output_path)
}
