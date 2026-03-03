use anyhow::Result as AnyResult;
use std::path::Path;
use teloxide::prelude::*;
use tokio::fs;

fn extract_extension(file_path: &str) -> &str {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
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

    #[test]
    fn ignores_dotted_directories_when_file_has_no_extension() {
        assert_eq!(extract_extension("videos.v1/source"), "bin");
    }

    #[test]
    fn extracts_extension_from_basename_with_dotted_directories() {
        assert_eq!(extract_extension("videos.v1/source.mkv"), "mkv");
    }
}
