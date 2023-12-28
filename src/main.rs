use dotenv::dotenv;
use std::{fs, process::Command};
use teloxide::{
    prelude::*,
    types::{InputFile, MediaKind, MessageKind},
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
        // Ensure the message is of a common type (text, media, etc.).
        let MessageKind::Common(common) = &msg.kind else {
            return respond(());
        };

        // Ensure the media in the message is a document.
        let MediaKind::Document(document) = &common.media_kind else {
            return respond(());
        };

        // Check if the document has a MIME type and retrieve it.
        let Some(mime_type) = &document.document.mime_type else {
            return respond(());
        };

        // Check if the MIME type of the document is 'video/webm'.
        if mime_type.essence_str() != "video/webm" {
            return respond(());
        };

        // Process the webm file and handle the result.
        match process_webm(&bot, &document.document.file.id, &msg).await {
            Ok(_) => {
                log::info!("Processed webm file")
            }
            Err(e) => {
                log::error!("Error processing webm file: {:?}", e)
            }
        }
        respond(())
    })
    .await;
}

/// Processes a `.webm` file by converting it to `.mp4` and sends it back via Telegram.
///
/// # Arguments
///
/// * `bot` - Reference to the Telegram Bot instance.
/// * `file_id` - The unique file identifier for the `.webm` file.
/// * `chat_id` - The chat ID where the message originated.
/// * `message_id` - The message ID of the original `.webm` file message.
///
/// # Returns
///
/// A `Result` indicating the success or failure of the processing.
async fn process_webm(
    bot: &Bot,
    file_id: &str,
    msg: &Message,
) -> Result<(), Box<dyn std::error::Error>> {
    let file_path = download_file(bot, file_id).await?;
    let converted_file_path = convert_webm_to_mp4(&file_path)?;

    let mut send_video_request = bot.send_video(msg.chat.id, InputFile::file(&converted_file_path));

    if let Some(thread_id) = msg.thread_id {
        send_video_request = send_video_request.message_thread_id(thread_id);
    }

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
