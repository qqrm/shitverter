use anyhow::Result as AnyResult;
use teloxide::prelude::*;
use tokio::fs;

/// Скачивает файл с серверов Telegram по его идентификатору.
pub async fn download_file(bot: &Bot, file_id: &str) -> AnyResult<String> {
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