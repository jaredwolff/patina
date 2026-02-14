use anyhow::Result;

use crate::base::Channel;

/// Coordinates the lifecycle of all enabled channels.
pub struct ChannelManager {
    channels: Vec<Box<dyn Channel>>,
}

impl ChannelManager {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
        }
    }

    pub fn register(&mut self, channel: Box<dyn Channel>) {
        tracing::info!("Registered channel: {}", channel.name());
        self.channels.push(channel);
    }

    pub fn enabled_channels(&self) -> Vec<&str> {
        self.channels.iter().map(|c| c.name()).collect()
    }

    pub async fn start_all(
        &mut self,
        _inbound_tx: tokio::sync::mpsc::Sender<nanobot_core::bus::InboundMessage>,
    ) -> Result<()> {
        // TODO: start all channels concurrently
        Ok(())
    }

    pub async fn stop_all(&mut self) -> Result<()> {
        for ch in &mut self.channels {
            ch.stop().await?;
        }
        Ok(())
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}
