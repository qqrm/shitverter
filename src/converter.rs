use anyhow::{anyhow, Result as AnyResult};
use std::process::Command;

/// Конвертирует любой поддерживаемый FFmpeg видеофайл в формат `.mp4`.
pub fn convert_video_to_mp4(file_path: &str) -> AnyResult<String> {
    let output_path = format!("{}.mp4", file_path);
    let output = Command::new("ffmpeg")
        .args(["-y", "-i", file_path, &output_path])
        .output()?;
    if !output.status.success() {
        return Err(anyhow!("FFmpeg conversion failed: {:?}", output));
    }
    Ok(output_path)
}
