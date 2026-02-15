use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use nanobot_core::bus::{InboundMessage, OutboundMessage};

/// Trait that all chat channel implementations must satisfy.
///
/// Methods take `&self` to allow wrapping in `Arc` for concurrent access
/// from the channel manager. Internal mutability should use `Mutex`/`RwLock`.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Channel name (e.g. "telegram", "discord").
    fn name(&self) -> &str;

    /// Start listening for messages. Sends inbound messages through the provided sender.
    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()>;

    /// Stop the channel and clean up resources.
    async fn stop(&self) -> Result<()>;

    /// Send a message through this channel.
    async fn send(&self, msg: &OutboundMessage) -> Result<()>;

    /// Check if a sender is allowed to use this bot.
    fn is_allowed(&self, sender_id: &str) -> bool;
}
