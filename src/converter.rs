use anyhow::{anyhow, Context, Result as AnyResult};
use std::path::Path;
use std::process::Command;

fn build_output_path(file_path: &str) -> String {
    let path = Path::new(file_path);
    let mut output_name = path
        .file_name()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "video".into());
    output_name.push(".converted.mp4");
    let output_path = path.with_file_name(output_name);
    output_path.to_string_lossy().to_string()
}

/// Конвертирует любой поддерживаемый FFmpeg видеофайл в формат `.mp4`.
pub fn convert_video_to_mp4(file_path: &str) -> AnyResult<String> {
    convert_with_command(file_path, &mut Command::new("ffmpeg"))
}

fn convert_with_command(file_path: &str, command: &mut Command) -> AnyResult<String> {
    let output_path = build_output_path(file_path);
    let output = command
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
    #[cfg(unix)]
    fn failed_converter_partial_output_is_removed_with_workspace() {
        let workspace = crate::media_workspace::MediaWorkspace::new().unwrap();
        let directory = workspace.path().to_path_buf();
        let input = directory.join("input.mkv");
        std::fs::write(&input, b"input").unwrap();
        // This deterministic stand-in writes the final command argument just as
        // FFmpeg can before failing; it does not need media or network access.
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "for output do :; done; printf partial > \"$output\"; exit 7",
            "fake-ffmpeg",
        ]);
        let result = super::convert_with_command(input.to_str().unwrap(), &mut command);
        assert!(result.unwrap_err().to_string().contains("status: 7"));
        assert!(directory.join("input.mkv.converted.mp4").exists());
        drop(workspace);
        assert!(!directory.exists());
    }

    #[test]
    fn keeps_output_distinct_from_input_with_existing_extension() {
        assert_eq!(
            build_output_path("/tmp/example.mkv"),
            "/tmp/example.mkv.converted.mp4"
        );
    }

    #[test]
    fn keeps_output_distinct_from_mp4_input() {
        assert_eq!(
            build_output_path("/tmp/example.mp4"),
            "/tmp/example.mp4.converted.mp4"
        );
    }

    #[test]
    fn appends_converted_mp4_when_extension_absent() {
        assert_eq!(
            build_output_path("/tmp/example"),
            "/tmp/example.converted.mp4"
        );
    }
}
