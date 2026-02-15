use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{broadcast, RwLock};
use tracing::info;

use crate::bus::OutboundMessage;
use crate::tools::Tool;

/// Tool for sending messages to chat channels.
pub struct MessageTool {
    outbound_tx: broadcast::Sender<OutboundMessage>,
    default_channel: Arc<RwLock<String>>,
    default_chat_id: Arc<RwLock<String>>,
}

impl MessageTool {
    pub fn new(outbound_tx: broadcast::Sender<OutboundMessage>) -> Self {
        Self {
            outbound_tx,
            default_channel: Arc::new(RwLock::new(String::new())),
            default_chat_id: Arc::new(RwLock::new(String::new())),
        }
    }

    /// Update the default routing context for this tool.
    pub async fn set_context(&self, channel: &str, chat_id: &str) {
        *self.default_channel.write().await = channel.to_string();
        *self.default_chat_id.write().await = chat_id.to_string();
    }
}

#[async_trait]
impl Tool for MessageTool {
    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to the user via a chat channel. Use this to proactively send messages \
         or notifications. The message will be delivered to the current channel/chat unless \
         overridden with explicit channel and chat_id parameters."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The message content to send"
                },
                "channel": {
                    "type": "string",
                    "description": "Target channel (e.g. 'telegram', 'cli'). Defaults to current channel."
                },
                "chat_id": {
                    "type": "string",
                    "description": "Target chat ID. Defaults to current chat."
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        let channel = match params.get("channel").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => self.default_channel.read().await.clone(),
        };

        let chat_id = match params.get("chat_id").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => self.default_chat_id.read().await.clone(),
        };

        if channel.is_empty() || chat_id.is_empty() {
            return Ok(
                "Error: No target channel/chat specified and no default context set.".into(),
            );
        }

        let msg = OutboundMessage {
            channel: channel.clone(),
            chat_id: chat_id.clone(),
            content: content.to_string(),
            reply_to: None,
            metadata: HashMap::new(),
        };

        match self.outbound_tx.send(msg) {
            Ok(_) => {
                info!("Message sent to {channel}:{chat_id}");
                Ok(format!("Message sent to {channel}:{chat_id}"))
            }
            Err(_) => {
                // No receivers (e.g. CLI mode) — message is logged but not delivered
                info!("Message logged (no active channel receivers): {channel}:{chat_id}");
                Ok(format!(
                    "Message logged to {channel}:{chat_id} (no active channel receivers)"
                ))
            }
        }
    }
}
