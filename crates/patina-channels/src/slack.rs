//! Slack channel implementation using slack-morphism with Socket Mode.
//!
//! Features:
//! - Socket Mode (WebSocket, no public endpoint needed)
//! - Markdown-to-mrkdwn conversion for rich responses
//! - Thread support via thread_ts
//! - File attachment URLs
//! - Allowlist access control

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use slack_morphism::prelude::*;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, error, info, warn};

use patina_config::SlackConfig;
use patina_core::bus::{InboundMessage, OutboundMessage};

use crate::base::Channel;
use crate::slack_markdown::markdown_to_slack_mrkdwn;

/// State passed to the Socket Mode push event handler via SlackClientEventsUserState.
struct SlackPushState {
    inbound_tx: mpsc::Sender<InboundMessage>,
    allow_from: Vec<String>,
    bot_token: SlackApiToken,
}

/// Slack channel using Socket Mode for receiving events and Web API for sending.
pub struct SlackChannel {
    config: SlackConfig,
    client: Arc<SlackHyperClient>,
    bot_token: SlackApiToken,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl SlackChannel {
    /// Create a new Slack channel from config.
    pub fn new(config: SlackConfig) -> Result<Self> {
        if config.app_token.is_empty() {
            return Err(anyhow::anyhow!("Slack app token (xapp-*) not configured"));
        }
        if config.bot_token.is_empty() {
            return Err(anyhow::anyhow!("Slack bot token (xoxb-*) not configured"));
        }

        let client = Arc::new(SlackClient::new(
            SlackClientHyperConnector::new()
                .map_err(|e| anyhow::anyhow!("Failed to create Slack HTTP connector: {e}"))?,
        ));

        let bot_token = SlackApiToken::new(config.bot_token.clone().into());

        Ok(Self {
            config,
            client,
            bot_token,
            shutdown_tx: Mutex::new(None),
        })
    }

    fn check_allowed(&self, sender_id: &str) -> bool {
        is_sender_allowed(sender_id, &self.config.allow_from)
    }
}

/// Push event handler — must be a plain function (not a closure) for slack-morphism.
/// State is accessed via SlackClientEventsUserState.
async fn handle_push_event(
    event: SlackPushEventCallback,
    client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let msg_event = match event.event {
        SlackEventCallbackBody::Message(ev) => ev,
        _ => return Ok(()),
    };

    // Skip bot's own messages
    if msg_event.sender.bot_id.is_some() {
        return Ok(());
    }

    // Skip message subtypes (edits, deletes, etc.) — only handle plain messages
    if msg_event.subtype.is_some() {
        return Ok(());
    }

    let user_id = match msg_event.sender.user {
        Some(ref uid) => uid.to_string(),
        None => return Ok(()),
    };

    let channel_id = match msg_event.origin.channel {
        Some(ref cid) => cid.to_string(),
        None => return Ok(()),
    };

    // Read our custom state
    let state_guard = states.read().await;
    let push_state = state_guard
        .get_user_state::<SlackPushState>()
        .expect("SlackPushState not found in user state");

    let allow_from = &push_state.allow_from;
    let inbound_tx = &push_state.inbound_tx;
    let bot_token = &push_state.bot_token;

    // Build sender_id: "U1234" or "U1234|username"
    let sender_id = if let Some(ref username) = msg_event.sender.username {
        format!("{user_id}|{username}")
    } else {
        // Try to look up the username via API
        let session = client.open_session(bot_token);
        match session
            .users_info(&SlackApiUsersInfoRequest::new(
                msg_event.sender.user.clone().unwrap(),
            ))
            .await
        {
            Ok(info) => {
                if let Some(ref name) = info.user.name {
                    format!("{user_id}|{name}")
                } else {
                    user_id.clone()
                }
            }
            Err(_) => user_id.clone(),
        }
    };

    // Check allowlist
    if !is_sender_allowed(&sender_id, allow_from) {
        warn!("Access denied for sender {sender_id} on Slack. Add to allowFrom to grant access.");
        return Ok(());
    }

    // Build chat_id with optional thread_ts
    let chat_id_str = if let Some(ref thread_ts) = msg_event.origin.thread_ts {
        format!("{channel_id}:{thread_ts}")
    } else {
        channel_id.clone()
    };

    // Extract message text
    let text = msg_event
        .content
        .as_ref()
        .and_then(|c| c.text.clone())
        .unwrap_or_default();

    if text.is_empty() {
        return Ok(());
    }

    // Extract file URLs
    let media: Vec<String> = msg_event
        .content
        .as_ref()
        .and_then(|c| c.files.as_ref())
        .map(|files| {
            files
                .iter()
                .filter_map(|f| f.permalink.as_ref().map(|u| u.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Build content with file references
    let mut content = text;
    for url in &media {
        content.push_str(&format!("\n[file: {url}]"));
    }

    debug!(
        "Slack message from {sender_id}: {}...",
        &content[..content.len().min(50)]
    );

    // Build metadata
    let mut metadata = HashMap::new();
    metadata.insert(
        "ts".to_string(),
        serde_json::Value::String(msg_event.origin.ts.to_string()),
    );
    if let Some(ref thread_ts) = msg_event.origin.thread_ts {
        metadata.insert(
            "thread_ts".to_string(),
            serde_json::Value::String(thread_ts.to_string()),
        );
    }
    metadata.insert("user_id".to_string(), serde_json::Value::String(user_id));
    metadata.insert(
        "channel_id".to_string(),
        serde_json::Value::String(channel_id),
    );

    let inbound = InboundMessage {
        channel: "slack".to_string(),
        sender_id,
        chat_id: chat_id_str,
        content,
        media,
        metadata,
        timestamp: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    };

    if let Err(e) = inbound_tx.send(inbound).await {
        error!("Failed to send inbound Slack message: {e}");
    }

    Ok(())
}

fn slack_error_handler(
    err: Box<dyn std::error::Error + Send + Sync>,
    _client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> HttpStatusCode {
    error!("Slack Socket Mode error: {err}");
    HttpStatusCode::OK
}

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        info!("Starting Slack bot (Socket Mode)...");

        // Verify bot identity
        let session = self.client.open_session(&self.bot_token);
        match session.auth_test().await {
            Ok(resp) => {
                info!(
                    "Slack bot @{} connected (team: {})",
                    resp.user.as_deref().unwrap_or("unknown"),
                    resp.team,
                );
            }
            Err(e) => {
                error!("Failed to verify Slack bot identity: {e}");
                return Err(anyhow::anyhow!("Failed to verify Slack bot identity: {e}"));
            }
        }

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        {
            let mut tx_guard = self.shutdown_tx.lock().await;
            *tx_guard = Some(shutdown_tx);
        }

        let client = self.client.clone();

        let callbacks = SlackSocketModeListenerCallbacks::new().with_push_events(handle_push_event);

        let push_state = SlackPushState {
            inbound_tx,
            allow_from: self.config.allow_from.clone(),
            bot_token: self.bot_token.clone(),
        };

        let listener_environment = Arc::new(
            SlackClientEventsListenerEnvironment::new(client.clone())
                .with_error_handler(slack_error_handler)
                .with_user_state(push_state),
        );

        let socket_mode_listener = SlackClientSocketModeListener::new(
            &SlackClientSocketModeConfig::new(),
            listener_environment,
            callbacks,
        );

        let app_token = SlackApiToken::new(self.config.app_token.clone().into());

        socket_mode_listener
            .listen_for(&app_token)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start Slack Socket Mode: {e}"))?;

        // Run serve in a task and wait for shutdown signal
        tokio::spawn(async move {
            tokio::select! {
                _ = socket_mode_listener.serve() => {
                    info!("Slack Socket Mode listener stopped");
                }
                _ = &mut shutdown_rx => {
                    info!("Slack bot shutdown signal received");
                }
            }
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Stopping Slack bot...");
        let mut tx_guard = self.shutdown_tx.lock().await;
        if let Some(tx) = tx_guard.take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        let (channel_id, thread_ts) = parse_chat_id(&msg.chat_id)?;

        let session = self.client.open_session(&self.bot_token);

        // Split into chunks if needed (Slack limit: 40,000 chars)
        let chunks = split_message(&msg.content, 40_000);

        for chunk in &chunks {
            let mrkdwn_content = markdown_to_slack_mrkdwn(chunk);

            let mut request = SlackApiChatPostMessageRequest::new(
                channel_id.clone().into(),
                SlackMessageContent::new().with_text(mrkdwn_content),
            );

            if let Some(ref ts) = thread_ts {
                request = request.with_thread_ts(ts.clone().into());
            }

            if let Err(e) = session.chat_post_message(&request).await {
                error!("Error sending Slack message: {e}");
                return Err(anyhow::anyhow!("Failed to send Slack message: {e}"));
            }
        }

        Ok(())
    }

    fn is_allowed(&self, sender_id: &str) -> bool {
        self.check_allowed(sender_id)
    }

    fn prompt_rules(&self) -> &str {
        self.config
            .system_prompt_rules
            .as_deref()
            .unwrap_or("No markdown tables. Never use markdown table syntax — Slack does not support table formatting. Use plain text lists instead.")
    }
}

/// Parse chat_id into channel ID and optional thread_ts.
///
/// Format: "C1234" or "C1234:1234567890.123456"
fn parse_chat_id(chat_id: &str) -> Result<(String, Option<String>)> {
    let parts: Vec<&str> = chat_id.splitn(2, ':').collect();
    let channel_id = parts[0].to_string();
    let thread_ts = parts.get(1).map(|s| s.to_string());
    Ok((channel_id, thread_ts))
}

/// Split a message into chunks that fit within Slack's character limit.
fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining);
            break;
        }

        let split_at = remaining[..max_len]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(max_len);

        chunks.push(&remaining[..split_at]);
        remaining = &remaining[split_at..];
    }

    chunks
}

/// Check if a sender is allowed based on the allow_from list.
///
/// Matches against the full sender_id string, the user ID part,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_matches_full_and_composite_parts() {
        assert!(is_sender_allowed("U123|alice", &[]));
        assert!(is_sender_allowed("U123|alice", &[String::from("U123")]));
        assert!(is_sender_allowed("U123|alice", &[String::from("alice")]));
        assert!(is_sender_allowed(
            "U123|alice",
            &[String::from("U123|alice")]
        ));
        assert!(!is_sender_allowed("U123|alice", &[String::from("U456")]));
    }

    #[test]
    fn allowlist_simple_id_without_pipe() {
        assert!(is_sender_allowed("U999", &[String::from("U999")]));
        assert!(!is_sender_allowed("U999", &[String::from("U111")]));
        assert!(is_sender_allowed("U999", &[]));
    }

    #[test]
    fn parse_chat_id_without_thread() {
        let (channel, thread) = parse_chat_id("C1234").unwrap();
        assert_eq!(channel, "C1234");
        assert!(thread.is_none());
    }

    #[test]
    fn parse_chat_id_with_thread() {
        let (channel, thread) = parse_chat_id("C1234:1234567890.123456").unwrap();
        assert_eq!(channel, "C1234");
        assert_eq!(thread, Some("1234567890.123456".to_string()));
    }

    #[test]
    fn split_message_short() {
        let chunks = split_message("hello", 40_000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn split_message_at_newline() {
        let line = "a".repeat(20_000);
        let text = format!("{line}\n{line}\n{line}");
        let chunks = split_message(&text, 40_001);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].ends_with('\n'));
    }

    #[test]
    fn split_message_no_newline() {
        let text = "a".repeat(50_000);
        let chunks = split_message(&text, 40_000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 40_000);
        assert_eq!(chunks[1].len(), 10_000);
    }

    #[test]
    fn new_channel_requires_app_token() {
        let config = SlackConfig {
            enabled: true,
            app_token: String::new(),
            bot_token: "xoxb-test".into(),
            ..Default::default()
        };
        let result = SlackChannel::new(config);
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("app token"));
    }

    #[test]
    fn new_channel_requires_bot_token() {
        let config = SlackConfig {
            enabled: true,
            app_token: "xapp-test".into(),
            bot_token: String::new(),
            ..Default::default()
        };
        let result = SlackChannel::new(config);
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("bot token"));
    }
}
