use dotenv::dotenv;
use std::{fs, process::Command};
use teloxide::{
    prelude::*,
    types::{InputFile, MediaKind, MessageKind, ParseMode},
};

// The main entry point for the async application.
#[tokio::main]
async fn main() {
    // Load environment variables from a `.env` file, if it exists.
    dotenv().ok();

    // Initialize the logger for debugging and information purposes.
    pretty_env_logger::init();
    log::info!("Starting bot");

    // Initialize the bot with the token from environment variables.
    let bot = Bot::from_env();

    // Start the bot and handle incoming messages.
    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        // Process the webm file and handle the result.
        let _ = process_webm(&bot, &msg)
            .await
            .map_err(|e| log::error!("Error processing webm file: {:?}", e));

        // Process new members and handle the result.
        let _ = process_new_member(&bot, &msg)
            .await
            .map_err(|e| log::error!("Error processing webm file: {:?}", e));

        respond(())
    })
    .await;
}

/// Processes the joining of a new member and sends a message with their IDs.
///
/// # Arguments
///
/// * `bot` - Reference to the Telegram Bot instance.
/// * `msg` - Reference to the incoming Telegram Message.
///
/// # Returns
///
/// A `Result` indicating the success or failure of the processing.
async fn process_new_member(bot: &Bot, msg: &Message) -> Result<(), Box<dyn std::error::Error>> {
    // Ensure the message is of a joining type
    let MessageKind::NewChatMembers(new_members_msg) = &msg.kind else {
        return Ok(());
    };

    // Build resp with ID's
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

    // Send response
    bot.send_message(msg.chat.id, resp_with_ids)
    .parse_mode(ParseMode::MarkdownV2).await?;

    Ok(())
}

/// Processes a `.webm` file by converting it to `.mp4` and sends it back via Telegram.
///
/// # Arguments
///
/// * `bot` - Reference to the Telegram Bot instance.
/// * `msg` - Reference to Message.
///
/// # Returns
///
/// A `Result` indicating the success or failure of the processing.
async fn process_webm(bot: &Bot, msg: &Message) -> Result<(), Box<dyn std::error::Error>> {
    // Ensure the message is of a common type (text, media, etc.).
    let MessageKind::Common(common) = &msg.kind else {
        return Ok(());
    };

    // Ensure the media in the message is a document.
    let MediaKind::Document(document) = &common.media_kind else {
        return Ok(());
    };

    // Check if the document has a MIME type and retrieve it.
    let Some(mime_type) = &document.document.mime_type else {
        return Ok(());
    };

    // Check if the MIME type of the document is 'video/webm'.
    if mime_type.essence_str() != "video/webm" {
        return Ok(());
    };

    let file_path = download_file(bot, &document.document.file.id).await?;
    let converted_file_path = convert_webm_to_mp4(&file_path)?;

    let mut send_video_request = bot.send_video(msg.chat.id, InputFile::file(&converted_file_path));
    send_video_request = send_video_request.disable_notification(true);

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

    if let Some(msg) = msg.reply_to_message() {
        send_video_request = send_video_request.reply_to_message_id(msg.id);
    }
    send_video_request = send_video_request.parse_mode(ParseMode::MarkdownV2);
    send_video_request.await?;
    bot.delete_message(msg.chat.id, msg.id).await?;

    // Clean up: delete the downloaded and converted files
    fs::remove_file(&file_path)?;
    fs::remove_file(&converted_file_path)?;

    Ok(())
}

/// Downloads a file from Telegram servers.
///
/// # Arguments
///
/// * `bot` - Reference to the Telegram Bot instance.
/// * `file_id` - The unique file identifier for the file to download.
///
/// # Returns
///
/// A `Result` containing the path to the downloaded file or an error.
async fn download_file(bot: &Bot, file_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Get the file from Telegram
    let file = bot.get_file(file_id).send().await?;

    // Construct the download URL
    let download_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot.token(),
        file.path
    );

    // Proceed to download the file
    let response = reqwest::get(&download_url).await?;
    let file_path = format!("/tmp/{}.webm", file_id);
    let mut file = std::fs::File::create(&file_path)?;
    let content = response.bytes().await?;
    std::io::copy(&mut content.as_ref(), &mut file)?;
    Ok(file_path)
}

/// Converts a `.webm` file to `.mp4` format using FFmpeg.
///
/// # Arguments
///
/// * `file_path` - The file path of the `.webm` file.
///
/// # Returns
///
/// A `Result` containing the path to the converted `.mp4` file or an error.
fn convert_webm_to_mp4(file_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Use FFmpeg to convert the file and return the new file path
    let output_path = format!("{}.mp4", file_path);
    Command::new("ffmpeg")
        .args(["-i", file_path, &output_path])
        .output()?;
    Ok(output_path)
}
