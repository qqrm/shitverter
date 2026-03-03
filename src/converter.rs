use anyhow::{anyhow, Context, Result as AnyResult};
use std::path::{Path, PathBuf};
use std::process::Command;

fn build_output_path(file_path: &str) -> String {
    let path = Path::new(file_path);
    let mut output_path = PathBuf::from(path);
    output_path.set_extension("mp4");
    output_path.to_string_lossy().to_string()
}

/// Конвертирует любой поддерживаемый FFmpeg видеофайл в формат `.mp4`.
pub fn convert_video_to_mp4(file_path: &str) -> AnyResult<String> {
    let output_path = build_output_path(file_path);
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-i",
            file_path,
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
            &output_path,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(anyhow!(
            "FFmpeg conversion failed (status: {}): stderr='{}' stdout='{}'",
            output.status.code().map_or_else(
                || "terminated by signal".to_string(),
                |code| code.to_string()
            ),
            stderr,
            stdout,
        ));
    }

    std::fs::metadata(&output_path)
        .with_context(|| format!("Converted file is missing: {}", output_path))?;

    Ok(output_path)
}

#[cfg(test)]
mod tests {
    use super::build_output_path;

    #[test]
    fn replaces_existing_extension_with_mp4() {
        assert_eq!(build_output_path("/tmp/example.mkv"), "/tmp/example.mp4");
    }

    #[test]
    fn appends_mp4_when_extension_absent() {
        assert_eq!(build_output_path("/tmp/example"), "/tmp/example.mp4");
    }
}
