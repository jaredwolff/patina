//! Voice transcription with a local-first strategy.
//!
//! Primary: local inference via `parakeet-rs` (NVIDIA Parakeet TDT, ONNX Runtime).
//! Fallback: Groq Whisper API (cloud).

pub mod audio;
pub mod groq;
pub mod local;

use anyhow::Result;
use tracing::{info, warn};

use patina_config::{TranscriptionConfig, TranscriptionMode};

/// Transcription backend trait.
#[async_trait::async_trait]
pub trait Transcriber: Send + Sync {
    /// Transcribe an audio file at the given path.
    async fn transcribe_file(&self, file_path: &str) -> Result<String>;
}

/// Tries local transcription first, falls back to Groq on error.
struct AutoTranscriber {
    local: Option<Box<dyn Transcriber>>,
    fallback: Option<Box<dyn Transcriber>>,
}

#[async_trait::async_trait]
impl Transcriber for AutoTranscriber {
    async fn transcribe_file(&self, file_path: &str) -> Result<String> {
        if let Some(ref local) = self.local {
            match local.transcribe_file(file_path).await {
                Ok(text) => return Ok(text),
                Err(e) => {
                    warn!("Local transcription failed, trying fallback: {e}");
                }
            }
        }
        if let Some(ref fallback) = self.fallback {
            return fallback.transcribe_file(file_path).await;
        }
        Err(anyhow::anyhow!("No transcription backend available"))
    }
}

/// Resolve the model path, expanding ~ to home directory.
fn resolve_model_path(config: &TranscriptionConfig) -> String {
    if let Some(ref path) = config.model_path {
        if path.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                return path.replacen('~', &home.to_string_lossy(), 1);
            }
        }
        path.clone()
    } else {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".patina")
            .join("models")
            .join("parakeet-tdt")
            .to_string_lossy()
            .to_string()
    }
}

/// Check if model files exist at the given path.
pub fn model_files_exist(model_path: &str) -> bool {
    let dir = std::path::Path::new(model_path);
    dir.join("encoder-model.onnx").exists()
        && dir.join("decoder_joint-model.onnx").exists()
        && dir.join("vocab.txt").exists()
}

fn download_missing_model_files(model_path: &str, base_url: &str) -> Result<()> {
    let dir = std::path::Path::new(model_path);
    std::fs::create_dir_all(dir)?;

    let base = base_url.trim_end_matches('/');
    let files = [
        "encoder-model.onnx",
        "encoder-model.onnx.data",
        "decoder_joint-model.onnx",
        "vocab.txt",
    ];

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    for file in files {
        let target = dir.join(file);
        if target.exists() {
            continue;
        }
        let url = format!("{base}/{file}");
        info!("Downloading Parakeet model file: {url}");
        let resp = client.get(&url).send()?.error_for_status()?;
        let bytes = resp.bytes()?;
        std::fs::write(&target, &bytes)?;
    }

    Ok(())
}

fn ensure_local_model_available(config: &TranscriptionConfig, model_path: &str) -> Result<bool> {
    if model_files_exist(model_path) {
        return Ok(true);
    }

    if !config.auto_download {
        return Ok(false);
    }

    let base = config
        .model_url
        .as_deref()
        .unwrap_or("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main");

    match download_missing_model_files(model_path, base) {
        Ok(()) => Ok(model_files_exist(model_path)),
        Err(e) => Err(e),
    }
}

/// Create a transcriber based on configuration.
///
/// - `mode: Local` — only local, error if model not found or ffmpeg missing
/// - `mode: Groq` — only Groq, error if no API key
/// - `mode: Auto` — try local first; if model not found, fall back to Groq only
pub fn create_transcriber(
    config: &TranscriptionConfig,
    groq_api_key: Option<String>,
) -> Result<Box<dyn Transcriber>> {
    let model_path = resolve_model_path(config);
    let ep = config.execution_provider.as_deref().unwrap_or("cpu");

    match config.mode {
        TranscriptionMode::Local => {
            if !ensure_local_model_available(config, &model_path)? {
                anyhow::bail!(
                    "Transcription mode is 'local' but local model files are missing at {model_path}"
                );
            }
            if !audio::ffmpeg_available() {
                anyhow::bail!("Transcription mode is 'local' but ffmpeg is not installed");
            }
            let local = try_create_local(&model_path, ep)?;
            Ok(Box::new(AutoTranscriber {
                local: Some(local),
                fallback: None,
            }))
        }
        TranscriptionMode::Groq => {
            let key = groq_api_key.filter(|k| !k.is_empty()).ok_or_else(|| {
                anyhow::anyhow!("Transcription mode is 'groq' but no Groq API key configured")
            })?;
            Ok(Box::new(groq::GroqTranscriber::new(key)))
        }
        TranscriptionMode::Auto => {
            let mut local_transcriber: Option<Box<dyn Transcriber>> = None;
            let mut fallback: Option<Box<dyn Transcriber>> = None;

            let local_model_available = match ensure_local_model_available(config, &model_path) {
                Ok(v) => v,
                Err(e) => {
                    warn!("Failed to auto-download local model files: {e}");
                    false
                }
            };

            // Try local
            if audio::ffmpeg_available() && local_model_available {
                match try_create_local(&model_path, ep) {
                    Ok(t) => {
                        info!("Local Parakeet transcription available");
                        local_transcriber = Some(t);
                    }
                    Err(e) => {
                        warn!("Failed to initialize local transcription: {e}");
                    }
                }
            } else {
                if !audio::ffmpeg_available() {
                    info!("ffmpeg not found, local transcription unavailable");
                }
                if !local_model_available {
                    info!(
                        "Parakeet model not found at {model_path}, local transcription unavailable"
                    );
                }
            }

            // Set up Groq fallback
            if let Some(key) = groq_api_key.filter(|k| !k.is_empty()) {
                fallback = Some(Box::new(groq::GroqTranscriber::new(key)));
            }

            if local_transcriber.is_none() && fallback.is_none() {
                info!("No transcription backend available (no local model, no Groq API key)");
            }

            Ok(Box::new(AutoTranscriber {
                local: local_transcriber,
                fallback,
            }))
        }
    }
}

/// Try to create a local transcriber. Returns an error if the parakeet feature
/// is not compiled in or if model loading fails.
fn try_create_local(model_path: &str, execution_provider: &str) -> Result<Box<dyn Transcriber>> {
    #[cfg(feature = "parakeet")]
    {
        let t = local::LocalTranscriber::new(model_path, execution_provider)?;
        Ok(Box::new(t))
    }
    #[cfg(not(feature = "parakeet"))]
    {
        let _ = (model_path, execution_provider);
        Err(anyhow::anyhow!(
            "Local transcription not available: built without 'parakeet' feature"
        ))
    }
}
