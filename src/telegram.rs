use anyhow::{anyhow, Context, Result as AnyResult};
use std::{
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};
use teloxide::prelude::*;
use tokio::{fs, io::AsyncWriteExt};

static DOWNLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn extract_extension(file_path: &str) -> &str {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("bin");

    if extension.len() <= 10 && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        extension
    } else {
        "bin"
    }
}

fn download_path(extension: &str) -> PathBuf {
    let sequence = DOWNLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "shitverter-{}-{}.{}",
        process::id(),
        sequence,
        extension
    ))
}

/// Скачивает файл с серверов Telegram по его идентификатору.
pub async fn download_file(bot: &Bot, file_id: &str, max_bytes: u64) -> AnyResult<String> {
    let file = bot.get_file(file_id.to_owned().into()).send().await?;

    if u64::from(file.size) > max_bytes {
        return Err(anyhow!(
            "Telegram file is too large: {} bytes exceeds configured limit of {} bytes",
            file.size,
            max_bytes,
        ));
    }

    let download_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot.token(),
        file.path
    );
    let extension = extract_extension(&file.path);
    let file_path = download_path(extension);
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file_path)
        .await
        .with_context(|| format!("Failed to create temporary file {}", file_path.display()))?;

    let download_result: AnyResult<()> = async {
        let mut response = reqwest::get(&download_url).await?.error_for_status()?;
        if response
            .content_length()
            .is_some_and(|content_length| content_length > max_bytes)
        {
            return Err(anyhow!(
                "Telegram download is too large: response advertises more than {} bytes",
                max_bytes,
            ));
        }

        let mut downloaded_bytes = 0_u64;
        while let Some(chunk) = response.chunk().await? {
            downloaded_bytes = downloaded_bytes
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| anyhow!("Telegram download size overflow"))?;
            if downloaded_bytes > max_bytes {
                return Err(anyhow!(
                    "Telegram download exceeds configured limit of {} bytes",
                    max_bytes,
                ));
            }
            destination.write_all(&chunk).await?;
        }
        destination.flush().await?;
        Ok(())
    }
    .await;

    if let Err(error) = download_result {
        drop(destination);
        if let Err(remove_error) = fs::remove_file(&file_path).await {
            log::warn!(
                "Failed to remove incomplete download {}: {:?}",
                file_path.display(),
                remove_error
            );
        }
        return Err(error);
    }

    Ok(file_path.to_string_lossy().into_owned())
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

    #[test]
    fn rejects_unsafe_extension() {
        assert_eq!(extract_extension("videos/source.mp4;rm"), "bin");
    }
}
