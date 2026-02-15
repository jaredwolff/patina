//! Groq Whisper API transcription backend.
//!
//! Uses Groq's OpenAI-compatible endpoint for cloud-based voice transcription.

use anyhow::{bail, Result};
use tracing::error;

use crate::Transcriber;

/// Cloud transcription backend via Groq's Whisper API.
pub struct GroqTranscriber {
    api_key: String,
}

impl GroqTranscriber {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait::async_trait]
impl Transcriber for GroqTranscriber {
    async fn transcribe_file(&self, file_path: &str) -> Result<String> {
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            bail!("Audio file not found: {file_path}");
        }

        let file_bytes = tokio::fs::read(path).await?;

        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Infer MIME type from extension
        let mime = match path.extension().and_then(|e| e.to_str()) {
            Some("ogg") => "audio/ogg",
            Some("mp3") => "audio/mpeg",
            Some("m4a") => "audio/mp4",
            Some("wav") => "audio/wav",
            _ => "audio/ogg",
        };

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str(mime)?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", "whisper-large-v3");

        let client = reqwest::Client::new();
        let resp = client
            .post("https://api.groq.com/openai/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("Groq transcription failed ({status}): {body}");
            bail!("Groq transcription failed ({status})");
        }

        let data: serde_json::Value = resp.json().await?;
        data.get("text")
            .and_then(|t| t.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("No text field in Groq response"))
    }
}
