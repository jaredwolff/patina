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
    #[serde(default = "default_timestamp")]
    pub timestamp: String,
}

pub fn default_timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_key_format() {
        let msg = InboundMessage {
            channel: "telegram".into(),
            sender_id: "user42".into(),
            chat_id: "12345".into(),
            content: "hello".into(),
            media: Vec::new(),
            metadata: HashMap::new(),
            timestamp: default_timestamp(),
        };
        assert_eq!(msg.session_key(), "telegram:12345");
    }

    #[test]
    fn test_session_key_with_special_chars() {
        let msg = InboundMessage {
            channel: "cli".into(),
            sender_id: "local".into(),
            chat_id: "interactive".into(),
            content: "".into(),
            media: Vec::new(),
            metadata: HashMap::new(),
            timestamp: default_timestamp(),
        };
        assert_eq!(msg.session_key(), "cli:interactive");
    }

    #[tokio::test]
    async fn test_inbound_send_receive() {
        let mut bus = MessageBus::new(16);
        let msg = InboundMessage {
            channel: "test".into(),
            sender_id: "u1".into(),
            chat_id: "c1".into(),
            content: "hello".into(),
            media: vec!["photo.jpg".into()],
            metadata: HashMap::new(),
            timestamp: default_timestamp(),
        };

        bus.inbound_tx.send(msg).await.unwrap();
        let received = bus.inbound_rx.recv().await.unwrap();
        assert_eq!(received.content, "hello");
        assert_eq!(received.media, vec!["photo.jpg"]);
    }

    #[tokio::test]
    async fn test_outbound_broadcast() {
        let bus = MessageBus::new(16);
        let mut rx1 = bus.outbound_tx.subscribe();
        let mut rx2 = bus.outbound_tx.subscribe();

        let msg = OutboundMessage {
            channel: "telegram".into(),
            chat_id: "99".into(),
            content: "response".into(),
            reply_to: None,
            metadata: HashMap::new(),
        };

        bus.outbound_tx.send(msg).unwrap();

        let r1 = rx1.recv().await.unwrap();
        let r2 = rx2.recv().await.unwrap();
        assert_eq!(r1.content, "response");
        assert_eq!(r2.content, "response");
    }

    #[test]
    fn test_inbound_message_serialization() {
        let msg = InboundMessage {
            channel: "test".into(),
            sender_id: "u".into(),
            chat_id: "c".into(),
            content: "hi".into(),
            media: Vec::new(),
            metadata: {
                let mut m = HashMap::new();
                m.insert("key".into(), serde_json::json!("value"));
                m
            },
            timestamp: default_timestamp(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "hi");
        assert_eq!(
            deserialized.metadata.get("key"),
            Some(&serde_json::json!("value"))
        );
    }
}
