use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{broadcast, mpsc};

/// Message received from a chat channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel: String,
    pub sender_id: String,
    pub chat_id: String,
    pub content: String,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl InboundMessage {
    pub fn session_key(&self) -> String {
        format!("{}:{}", self.channel, self.chat_id)
    }
}

/// Message to send to a chat channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Async message bus connecting channels to the agent.
pub struct MessageBus {
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    pub inbound_rx: mpsc::Receiver<InboundMessage>,
    pub outbound_tx: broadcast::Sender<OutboundMessage>,
}

impl MessageBus {
    pub fn new(buffer: usize) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(buffer);
        let (outbound_tx, _) = broadcast::channel(buffer);
        Self {
            inbound_tx,
            inbound_rx,
            outbound_tx,
        }
    }
}
