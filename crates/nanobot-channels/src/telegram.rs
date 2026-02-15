//! Telegram channel implementation using teloxide.
//!
//! Features:
//! - Long polling (no webhook/public IP needed)
//! - Markdown-to-HTML conversion for rich responses
//! - Thread/topic support for group chats
//! - Photo/voice/document handling (downloads to ~/.nanobot/media/)
//! - Typing indicators while processing
//! - Proxy support
//! - /start, /new, /help slash commands

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    BotCommand, ChatAction, FileMeta, MediaKind, MessageKind, ParseMode, ThreadId,
};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, error, info, warn};

use nanobot_config::TelegramConfig;
use nanobot_core::bus::{InboundMessage, OutboundMessage};

use crate::base::Channel;
use crate::markdown::markdown_to_telegram_html;

/// Telegram channel using long polling.
pub struct TelegramChannel {
    config: TelegramConfig,
    bot: Bot,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    typing_tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

impl TelegramChannel {
    /// Create a new Telegram channel from config.
    pub fn new(config: TelegramConfig) -> Result<Self> {
        if config.token.is_empty() {
            return Err(anyhow::anyhow!("Telegram bot token not configured"));
        }

        // Build bot with optional proxy
        let bot = match config.proxy.as_deref() {
            Some(proxy_url) if !proxy_url.is_empty() => {
                let client = reqwest::Client::builder()
                    .proxy(reqwest::Proxy::all(proxy_url)?)
                    .build()?;
                Bot::with_client(&config.token, client)
            }
            _ => Bot::new(&config.token),
        };

        Ok(Self {
            config,
            bot,
            shutdown_tx: Mutex::new(None),
            typing_tasks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Stop the typing indicator for a chat.
    async fn stop_typing(&self, chat_id_str: &str) {
        let mut tasks = self.typing_tasks.lock().await;
        if let Some(handle) = tasks.remove(chat_id_str) {
            handle.abort();
        }
    }

    /// Check if a sender is allowed.
    fn check_allowed(&self, sender_id: &str) -> bool {
        is_sender_allowed(sender_id, &self.config.allow_from)
    }

    /// Get file extension based on media type and optional MIME type.
    fn get_extension(media_type: &str, mime_type: Option<&str>) -> &'static str {
        if let Some(mime) = mime_type {
            match mime {
                "image/jpeg" => return ".jpg",
                "image/png" => return ".png",
                "image/gif" => return ".gif",
                "audio/ogg" => return ".ogg",
                "audio/mpeg" => return ".mp3",
                "audio/mp4" => return ".m4a",
                _ => {}
            }
        }

        match media_type {
            "image" => ".jpg",
            "voice" => ".ogg",
            "audio" => ".mp3",
            _ => "",
        }
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        info!("Starting Telegram bot (polling mode)...");

        // Register bot commands
        let commands = vec![
            BotCommand::new("start", "Start the bot"),
            BotCommand::new("new", "Start a new conversation"),
            BotCommand::new("help", "Show available commands"),
        ];
        if let Err(e) = self.bot.set_my_commands(commands).await {
            warn!("Failed to register bot commands: {e}");
        }

        // Get bot info
        match self.bot.get_me().await {
            Ok(me) => {
                info!(
                    "Telegram bot @{} connected",
                    me.username.as_deref().unwrap_or("unknown")
                );
            }
            Err(e) => {
                error!("Failed to get bot info: {e}");
            }
        }

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        {
            let mut tx_guard = self.shutdown_tx.lock().await;
            *tx_guard = Some(shutdown_tx);
        }

        let bot = self.bot.clone();
        let config = self.config.clone();
        let typing_tasks = self.typing_tasks.clone();

        // Delete webhook to ensure polling works
        if let Err(e) = bot.delete_webhook().await {
            warn!("Failed to delete webhook: {e}");
        }

        // Build the handler
        let handler = Update::filter_message().endpoint(
            move |bot: Bot, msg: Message, inbound_tx: mpsc::Sender<InboundMessage>| {
                let config = config.clone();
                let typing_tasks = typing_tasks.clone();
                async move {
                    handle_message(bot, msg, inbound_tx, config, typing_tasks).await;
                    respond(())
                }
            },
        );

        // Build dispatcher
        let mut dispatcher = Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![inbound_tx])
            .default_handler(|_upd| async {})
            .error_handler(LoggingErrorHandler::with_custom_text(
                "Error in telegram handler",
            ))
            .build();

        // Run polling with shutdown support
        let shutdown_token = dispatcher.shutdown_token();
        tokio::spawn(async move {
            let _ = shutdown_rx.await;
            match shutdown_token.shutdown() {
                Ok(fut) => fut.await,
                Err(e) => warn!("Failed to shutdown dispatcher: {e:?}"),
            }
        });

        dispatcher.dispatch().await;

        info!("Telegram bot stopped");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Stopping Telegram bot...");

        // Cancel all typing indicators
        {
            let mut tasks = self.typing_tasks.lock().await;
            for (_, handle) in tasks.drain() {
                handle.abort();
            }
        }

        // Send shutdown signal
        let mut tx_guard = self.shutdown_tx.lock().await;
        if let Some(tx) = tx_guard.take() {
            let _ = tx.send(());
        }

        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        // Stop typing indicator
        self.stop_typing(&msg.chat_id).await;

        // Parse chat_id — format is "chat_id" or "chat_id:thread_id"
        let parts: Vec<&str> = msg.chat_id.splitn(2, ':').collect();
        let chat_id: i64 = parts[0]
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid chat_id: {}", msg.chat_id))?;
        let thread_id: Option<ThreadId> = parts
            .get(1)
            .and_then(|s| s.parse::<i32>().ok())
            .map(|id| ThreadId(teloxide::types::MessageId(id)));

        // Also check metadata for thread_id as fallback
        let thread_id = thread_id.or_else(|| {
            msg.metadata
                .get("message_thread_id")
                .and_then(|v| v.as_i64())
                .map(|v| ThreadId(teloxide::types::MessageId(v as i32)))
        });

        // Try sending as HTML first
        let html_content = markdown_to_telegram_html(&msg.content);
        let mut request = self
            .bot
            .send_message(ChatId(chat_id), &html_content)
            .parse_mode(ParseMode::Html);

        if let Some(tid) = thread_id {
            request = request.message_thread_id(tid);
        }

        match request.await {
            Ok(_) => {}
            Err(e) => {
                // Fallback to plain text if HTML parsing fails
                warn!("HTML parse failed, falling back to plain text: {e}");
                let mut fallback = self.bot.send_message(ChatId(chat_id), &msg.content);
                if let Some(tid) = thread_id {
                    fallback = fallback.message_thread_id(tid);
                }
                if let Err(e2) = fallback.await {
                    error!("Error sending Telegram message: {e2}");
                    return Err(e2.into());
                }
            }
        }

        Ok(())
    }

    fn is_allowed(&self, sender_id: &str) -> bool {
        self.check_allowed(sender_id)
    }
}

/// Handle an incoming Telegram message.
async fn handle_message(
    bot: Bot,
    msg: Message,
    inbound_tx: mpsc::Sender<InboundMessage>,
    config: TelegramConfig,
    typing_tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
) {
    // Extract user info
    let user = match msg.from {
        Some(ref u) => u,
        None => return,
    };

    // Build sender_id: "numeric_id" or "numeric_id|username"
    let sender_id = if let Some(ref username) = user.username {
        format!("{}|{}", user.id, username)
    } else {
        user.id.to_string()
    };

    // Check access
    if !is_sender_allowed(&sender_id, &config.allow_from) {
        warn!(
            "Access denied for sender {} on Telegram. Add to allowFrom to grant access.",
            sender_id
        );
        return;
    }

    // Build chat_id with optional thread_id
    let chat_id = msg.chat.id.0;
    let thread_id = msg.thread_id;
    let chat_id_str = if let Some(tid) = thread_id {
        format!("{}:{}", chat_id, (tid.0).0)
    } else {
        chat_id.to_string()
    };

    // Build content from text and/or media
    let mut content_parts: Vec<String> = Vec::new();
    let mut media_paths: Vec<String> = Vec::new();

    // Handle commands (text starting with /)
    if let Some(ref text) = msg.text() {
        content_parts.push(text.to_string());
    }
    if let Some(caption) = msg.caption() {
        content_parts.push(caption.to_string());
    }

    // Handle media
    if let MessageKind::Common(ref common) = msg.kind {
        match &common.media_kind {
            MediaKind::Photo(photo) => {
                // Download largest photo
                if let Some(largest) = photo.photo.last() {
                    match download_media(&bot, &largest.file, "image", None).await {
                        Ok(path) => {
                            media_paths.push(path.clone());
                            content_parts.push(format!("[image: {path}]"));
                        }
                        Err(e) => {
                            error!("Failed to download photo: {e}");
                            content_parts.push("[image: download failed]".to_string());
                        }
                    }
                }
            }
            MediaKind::Voice(voice) => {
                match download_media(
                    &bot,
                    &voice.voice.file,
                    "voice",
                    voice.voice.mime_type.as_ref().map(|m| m.as_ref()),
                )
                .await
                {
                    Ok(path) => {
                        media_paths.push(path.clone());
                        // Transcription deferred to Phase 4
                        content_parts.push(format!("[voice: {path}]"));
                    }
                    Err(e) => {
                        error!("Failed to download voice: {e}");
                        content_parts.push("[voice: download failed]".to_string());
                    }
                }
            }
            MediaKind::Audio(audio) => {
                match download_media(
                    &bot,
                    &audio.audio.file,
                    "audio",
                    audio.audio.mime_type.as_ref().map(|m| m.as_ref()),
                )
                .await
                {
                    Ok(path) => {
                        media_paths.push(path.clone());
                        content_parts.push(format!("[audio: {path}]"));
                    }
                    Err(e) => {
                        error!("Failed to download audio: {e}");
                        content_parts.push("[audio: download failed]".to_string());
                    }
                }
            }
            MediaKind::Document(doc) => {
                match download_media(
                    &bot,
                    &doc.document.file,
                    "file",
                    doc.document.mime_type.as_ref().map(|m| m.as_ref()),
                )
                .await
                {
                    Ok(path) => {
                        media_paths.push(path.clone());
                        content_parts.push(format!("[file: {path}]"));
                    }
                    Err(e) => {
                        error!("Failed to download document: {e}");
                        content_parts.push("[file: download failed]".to_string());
                    }
                }
            }
            MediaKind::Text(_) => {
                // Already handled via msg.text() above
            }
            _ => {
                // Other media types not handled yet
            }
        }
    }

    let content = if content_parts.is_empty() {
        "[empty message]".to_string()
    } else {
        content_parts.join("\n")
    };

    debug!(
        "Telegram message from {sender_id}: {}...",
        &content[..content.len().min(50)]
    );

    // Start typing indicator
    {
        let real_chat_id: i64 = chat_id_str
            .split(':')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        if real_chat_id != 0 {
            let bot_clone = bot.clone();
            let key = chat_id_str.clone();
            let typing_tasks_clone = typing_tasks.clone();

            // Cancel existing
            {
                let mut tasks = typing_tasks.lock().await;
                if let Some(old) = tasks.remove(&key) {
                    old.abort();
                }
            }

            let handle = tokio::spawn(async move {
                loop {
                    if let Err(e) = bot_clone
                        .send_chat_action(ChatId(real_chat_id), ChatAction::Typing)
                        .await
                    {
                        debug!("Typing indicator stopped for {key}: {e}");
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                }
            });

            let mut tasks = typing_tasks_clone.lock().await;
            tasks.insert(chat_id_str.clone(), handle);
        }
    }

    // Build metadata
    let mut metadata = HashMap::new();
    metadata.insert(
        "message_id".to_string(),
        serde_json::Value::Number(msg.id.0.into()),
    );
    if let Some(tid) = thread_id {
        metadata.insert(
            "message_thread_id".to_string(),
            serde_json::Value::Number((tid.0).0.into()),
        );
    }
    metadata.insert(
        "user_id".to_string(),
        serde_json::Value::Number(serde_json::Number::from(user.id.0)),
    );
    if let Some(ref username) = user.username {
        metadata.insert(
            "username".to_string(),
            serde_json::Value::String(username.clone()),
        );
    }
    metadata.insert(
        "first_name".to_string(),
        serde_json::Value::String(user.first_name.clone()),
    );
    metadata.insert(
        "is_group".to_string(),
        serde_json::Value::Bool(msg.chat.is_group() || msg.chat.is_supergroup()),
    );

    // Send to agent via inbound channel
    let inbound = InboundMessage {
        channel: "telegram".to_string(),
        sender_id,
        chat_id: chat_id_str,
        content,
        media: media_paths,
        metadata,
    };

    if let Err(e) = inbound_tx.send(inbound).await {
        error!("Failed to send inbound message: {e}");
    }
}

/// Check if a sender is allowed based on the allow_from list.
///
/// Matches against the full sender_id string, the numeric ID part,
/// and the username part (for composite "id|username" format).
fn is_sender_allowed(sender_id: &str, allow_from: &[String]) -> bool {
    if allow_from.is_empty() {
        return true;
    }

    if allow_from.contains(&sender_id.to_string()) {
        return true;
    }

    // Handle composite "id|username" format
    if sender_id.contains('|') {
        for part in sender_id.split('|') {
            if !part.is_empty() && allow_from.contains(&part.to_string()) {
                return true;
            }
        }
    }

    false
}

/// Download a media file from Telegram to ~/.nanobot/media/.
async fn download_media(
    bot: &Bot,
    file_meta: &FileMeta,
    media_type: &str,
    mime_type: Option<&str>,
) -> Result<String> {
    let file = bot.get_file(file_meta.id.clone()).await?;

    let ext = TelegramChannel::get_extension(media_type, mime_type);
    let id_str = &file_meta.id.0;
    let short_id = &id_str[..id_str.len().min(16)];

    let media_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nanobot")
        .join("media");
    std::fs::create_dir_all(&media_dir)?;

    let file_path = media_dir.join(format!("{short_id}{ext}"));

    // Download file
    let mut dst = tokio::fs::File::create(&file_path).await?;
    bot.download_file(&file.path, &mut dst).await?;

    debug!("Downloaded {media_type} to {}", file_path.display());
    Ok(file_path.to_string_lossy().to_string())
}
