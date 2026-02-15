//! Audio format conversion via ffmpeg.
//!
//! Converts any audio file (OGG Opus, MP3, M4A, etc.) to 16kHz mono WAV
//! suitable for Parakeet inference.

use std::path::Path;

use anyhow::{bail, Result};
use tokio::process::Command;

/// Check if ffmpeg is available on the system.
pub fn ffmpeg_available() -> bool {
    which::which("ffmpeg").is_ok()
}

/// Convert any audio file to 16kHz mono WAV suitable for Parakeet.
///
/// Returns the path to the converted WAV file (placed alongside the input).
/// The caller is responsible for cleaning up the temporary file.
pub async fn convert_to_wav_16k(input_path: &str) -> Result<String> {
    let input = Path::new(input_path);
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let output = input.with_file_name(format!("{stem}_16k.wav"));
    let output_str = output.to_string_lossy().to_string();

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            input_path,
            "-ar",
            "16000",
            "-ac",
            "1",
            "-loglevel",
            "error",
            &output_str,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .await?;

    if !status.success() {
        bail!("ffmpeg conversion failed with status: {status}");
    }

    Ok(output_str)
}
