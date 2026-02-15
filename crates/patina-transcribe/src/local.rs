//! Local transcription backend using NVIDIA Parakeet TDT via ONNX Runtime.
//!
//! The ParakeetTDT model is `!Send` (its ExecutionConfig contains `Rc`),
//! so it runs on a dedicated OS thread. Async callers communicate via channels.

#[cfg(feature = "parakeet")]
pub use inner::LocalTranscriber;

#[cfg(feature = "parakeet")]
mod inner {
    use anyhow::Result;
    use tokio::sync::{mpsc, oneshot};
    use tracing::info;

    use crate::audio::convert_to_wav_16k;

    /// Request sent to the worker thread.
    struct TranscribeRequest {
        wav_path: String,
        reply_tx: oneshot::Sender<Result<String>>,
    }

    /// Local transcription backend using ParakeetTDT.
    ///
    /// This is `Send + Sync` because it only holds a channel sender.
    /// The actual model lives on a dedicated OS thread.
    pub struct LocalTranscriber {
        request_tx: mpsc::Sender<TranscribeRequest>,
    }

    impl LocalTranscriber {
        /// Initialize the local transcriber.
        ///
        /// Spawns a dedicated OS thread that loads the model and processes
        /// requests sequentially. Returns an error if model loading fails.
        pub fn new(model_path: &str, execution_provider: &str) -> Result<Self> {
            let (request_tx, request_rx) = mpsc::channel::<TranscribeRequest>(32);
            let model_path = model_path.to_string();
            let ep = execution_provider.to_string();

            // Report whether model loading succeeded
            let (init_tx, init_rx) = std::sync::mpsc::channel::<Result<()>>();

            std::thread::Builder::new()
                .name("parakeet-worker".into())
                .spawn(move || {
                    worker_thread(model_path, ep, request_rx, init_tx);
                })?;

            // Block waiting for model initialization result (happens once at startup)
            match init_rx.recv() {
                Ok(Ok(())) => {
                    info!("Local Parakeet TDT model loaded successfully");
                    Ok(Self { request_tx })
                }
                Ok(Err(e)) => Err(e),
                Err(_) => Err(anyhow::anyhow!(
                    "Parakeet worker thread died during initialization"
                )),
            }
        }
    }

    fn worker_thread(
        model_path: String,
        execution_provider: String,
        mut request_rx: mpsc::Receiver<TranscribeRequest>,
        init_tx: std::sync::mpsc::Sender<Result<()>>,
    ) {
        #[allow(unused_imports)]
        use parakeet_rs::{ExecutionConfig, ExecutionProvider, ParakeetTDT, Transcriber};

        // Build execution config
        let config = match execution_provider.as_str() {
            #[cfg(feature = "cuda")]
            "cuda" => Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda)),
            #[cfg(feature = "migraphx")]
            "migraphx" => {
                Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::MiGraphX))
            }
            #[cfg(feature = "tensorrt")]
            "tensorrt" => {
                Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::TensorRt))
            }
            "cpu" | "" => None,
            other => {
                info!("Unknown execution provider '{other}', falling back to CPU");
                None
            }
        };

        // Load model
        let mut model = match ParakeetTDT::from_pretrained(&model_path, config) {
            Ok(m) => {
                let _ = init_tx.send(Ok(()));
                m
            }
            Err(e) => {
                let _ = init_tx.send(Err(anyhow::anyhow!(
                    "Failed to load Parakeet model from {model_path}: {e}"
                )));
                return;
            }
        };

        // Process requests sequentially
        while let Some(req) = request_rx.blocking_recv() {
            let result = model
                .transcribe_file(&req.wav_path, None)
                .map(|r| r.text.trim().to_string())
                .map_err(|e| anyhow::anyhow!("Transcription failed: {e}"));

            // Clean up the temporary WAV file
            let _ = std::fs::remove_file(&req.wav_path);

            let _ = req.reply_tx.send(result);
        }

        info!("Parakeet worker thread shutting down");
    }

    #[async_trait::async_trait]
    impl crate::Transcriber for LocalTranscriber {
        async fn transcribe_file(&self, file_path: &str) -> Result<String> {
            // Convert to 16kHz mono WAV via ffmpeg
            let wav_path = convert_to_wav_16k(file_path).await?;

            // Send to worker thread
            let (reply_tx, reply_rx) = oneshot::channel();
            self.request_tx
                .send(TranscribeRequest { wav_path, reply_tx })
                .await
                .map_err(|_| anyhow::anyhow!("Parakeet worker thread not running"))?;

            // Await result
            reply_rx
                .await
                .map_err(|_| anyhow::anyhow!("Parakeet worker thread dropped response"))?
        }
    }
}
