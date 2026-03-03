use anyhow::Result as AnyResult;
use teloxide::prelude::*;
use tokio::fs;

fn extract_extension(file_path: &str) -> &str {
    file_path
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or("bin")
}

/// Скачивает файл с серверов Telegram по его идентификатору.
pub async fn download_file(bot: &Bot, file_id: &str) -> AnyResult<String> {
    let file = bot.get_file(file_id).send().await?;
    let download_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot.token(),
        file.path
    );
    let response = reqwest::get(&download_url).await?;
    let extension = extract_extension(&file.path);
    let file_path = format!("/tmp/{}.{}", file_id, extension);
    let content = response.bytes().await?;
    fs::write(&file_path, &content).await?;
    Ok(file_path)
}

#[cfg(test)]
mod tests {
    use super::extract_extension;

    #[test]
    fn extracts_extension_from_path() {
        assert_eq!(extract_extension("videos/source.mkv"), "mkv");
    }

    #[test]
    fn falls_back_to_bin_without_extension() {
        assert_eq!(extract_extension("videos/source"), "bin");
    }
}
