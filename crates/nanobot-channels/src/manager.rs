//! Channel manager: coordinates the lifecycle of all enabled channels
//! and dispatches outbound messages to the appropriate channel.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use nanobot_core::bus::{InboundMessage, OutboundMessage};

use crate::base::Channel;

/// Coordinates the lifecycle of all enabled channels and dispatches
/// outbound messages to the appropriate channel by name.
pub struct ChannelManager {
    channels: Arc<RwLock<HashMap<String, Arc<dyn Channel>>>>,
    outbound_rx: Option<broadcast::Receiver<OutboundMessage>>,
    dispatch_handle: Option<JoinHandle<()>>,
}

impl ChannelManager {
    /// Create a new channel manager with an outbound message receiver.
    pub fn new(outbound_rx: broadcast::Receiver<OutboundMessage>) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            outbound_rx: Some(outbound_rx),
            dispatch_handle: None,
        }
    }

    /// Register a channel. Must be called before `start_all()`.
    pub async fn register(&self, channel: Arc<dyn Channel>) {
        let name = channel.name().to_string();
        info!("Registered channel: {name}");
        let mut channels = self.channels.write().await;
        channels.insert(name, channel);
    }

    /// List the names of all registered channels.
    pub async fn enabled_channels(&self) -> Vec<String> {
        let channels = self.channels.read().await;
        channels.keys().cloned().collect()
    }

    /// Start all channels and the outbound dispatcher.
    ///
    /// Each channel's `start()` is spawned as a separate task.
    /// The outbound dispatcher runs in another task, routing outbound
    /// messages to the appropriate channel by name.
    pub async fn start_all(&mut self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        // Start each channel
        let channels = self.channels.read().await;
        for (name, channel) in channels.iter() {
            let ch = channel.clone();
            let tx = inbound_tx.clone();
            let ch_name = name.clone();
            tokio::spawn(async move {
                if let Err(e) = ch.start(tx).await {
                    error!("Channel {ch_name} failed: {e}");
                }
            });
        }
        drop(channels);

        // Start outbound dispatcher
        if let Some(outbound_rx) = self.outbound_rx.take() {
            let channels = self.channels.clone();
            self.dispatch_handle = Some(tokio::spawn(async move {
                dispatch_outbound(outbound_rx, channels).await;
            }));
        }

        Ok(())
    }

    /// Stop all channels and the outbound dispatcher.
    pub async fn stop_all(&self) -> Result<()> {
        let channels = self.channels.read().await;
        for (name, channel) in channels.iter() {
            info!("Stopping channel: {name}");
            if let Err(e) = channel.stop().await {
                warn!("Error stopping channel {name}: {e}");
            }
        }
        Ok(())
    }
}

/// Outbound dispatcher loop: receives outbound messages from the bus
/// and routes them to the appropriate channel by name.
async fn dispatch_outbound(
    mut outbound_rx: broadcast::Receiver<OutboundMessage>,
    channels: Arc<RwLock<HashMap<String, Arc<dyn Channel>>>>,
) {
    loop {
        match outbound_rx.recv().await {
            Ok(msg) => {
                let channels = channels.read().await;
                if let Some(channel) = channels.get(&msg.channel) {
                    if let Err(e) = channel.send(&msg).await {
                        error!("Error sending to channel {}: {e}", msg.channel);
                    }
                } else {
                    // For CLI mode or system messages, just log
                    if msg.channel != "cli" && msg.channel != "system" {
                        warn!("No channel registered for: {}", msg.channel);
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("Outbound dispatcher lagged, missed {n} messages");
            }
            Err(broadcast::error::RecvError::Closed) => {
                info!("Outbound channel closed, dispatcher shutting down");
                break;
            }
        }
    }
}
