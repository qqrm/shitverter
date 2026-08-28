use anyhow::{anyhow, Context, Result as AnyResult};
use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};
use teloxide::{prelude::*, ApiError, RequestError};
use tokio::{fs, io::AsyncWriteExt};

static DOWNLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct FileTooLargeError {
    pub limit_bytes: Option<u64>,
}

impl fmt::Display for FileTooLargeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.limit_bytes {
            Some(limit_bytes) => write!(
                formatter,
                "file exceeds the configured download limit of {limit_bytes} bytes"
            ),
            None => {
                formatter.write_str("Telegram refused to provide the file because it is too large")
            }
        }
    }
}

impl Error for FileTooLargeError {}

fn is_telegram_file_too_large(error: &RequestError) -> bool {
    match error {
        RequestError::Api(ApiError::RequestEntityTooLarge) => true,
        RequestError::Api(ApiError::Unknown(message)) => {
            message.to_ascii_lowercase().contains("file is too big")
        }
        _ => false,
    }
}

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
    let file = bot
        .get_file(file_id.to_owned().into())
        .send()
        .await
        .map_err(|error| {
            if is_telegram_file_too_large(&error) {
                anyhow::Error::new(FileTooLargeError { limit_bytes: None })
            } else {
                anyhow::Error::new(error)
            }
        })?;

    if u64::from(file.size) > max_bytes {
        return Err(FileTooLargeError {
            limit_bytes: Some(max_bytes),
        }
        .into());
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
            return Err(FileTooLargeError {
                limit_bytes: Some(max_bytes),
            }
            .into());
        }

        let mut downloaded_bytes = 0_u64;
        while let Some(chunk) = response.chunk().await? {
            downloaded_bytes = downloaded_bytes
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| anyhow!("Telegram download size overflow"))?;
            if downloaded_bytes > max_bytes {
                return Err(FileTooLargeError {
                    limit_bytes: Some(max_bytes),
                }
                .into());
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
    use super::{extract_extension, is_telegram_file_too_large};
    use teloxide::{ApiError, RequestError};

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

    #[test]
    fn recognizes_telegram_file_too_big_error() {
        let error = RequestError::Api(ApiError::Unknown(
            "Bad Request: file is too big".to_string(),
        ));
        assert!(is_telegram_file_too_large(&error));
        assert!(is_telegram_file_too_large(&RequestError::Api(
            ApiError::RequestEntityTooLarge,
        )));
    }

    #[test]
    fn does_not_classify_unrelated_telegram_error_as_too_large() {
        let error = RequestError::Api(ApiError::Unknown(
            "Bad Request: invalid file_id".to_string(),
        ));
        assert!(!is_telegram_file_too_large(&error));
    }
}
