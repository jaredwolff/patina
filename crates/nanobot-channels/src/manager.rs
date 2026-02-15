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
    pub async fn stop_all(&mut self) -> Result<()> {
        if let Some(handle) = self.dispatch_handle.take() {
            handle.abort();
            info!("Stopped outbound dispatcher");
        }

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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    struct MockChannel {
        name: String,
        starts: AtomicUsize,
        stops: AtomicUsize,
        sends: AtomicUsize,
    }

    impl MockChannel {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                starts: AtomicUsize::new(0),
                stops: AtomicUsize::new(0),
                sends: AtomicUsize::new(0),
            }
        }

        fn starts(&self) -> usize {
            self.starts.load(Ordering::SeqCst)
        }

        fn stops(&self) -> usize {
            self.stops.load(Ordering::SeqCst)
        }

        fn sends(&self) -> usize {
            self.sends.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            &self.name
        }

        async fn start(&self, _inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&self) -> Result<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send(&self, _msg: &OutboundMessage) -> Result<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn is_allowed(&self, _sender_id: &str) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn manager_routes_outbound_and_stops_all() {
        let (outbound_tx, outbound_rx) = broadcast::channel(16);
        let mut manager = ChannelManager::new(outbound_rx);

        let ch = Arc::new(MockChannel::new("telegram"));
        let ch_dyn: Arc<dyn Channel> = ch.clone();
        manager.register(ch_dyn).await;

        let (inbound_tx, _inbound_rx) = mpsc::channel(16);
        manager.start_all(inbound_tx).await.unwrap();
        sleep(Duration::from_millis(50)).await;

        outbound_tx
            .send(OutboundMessage {
                channel: "telegram".to_string(),
                chat_id: "1".to_string(),
                content: "hello".to_string(),
                metadata: HashMap::new(),
            })
            .unwrap();
        sleep(Duration::from_millis(50)).await;

        assert_eq!(ch.starts(), 1);
        assert_eq!(ch.sends(), 1);

        manager.stop_all().await.unwrap();
        assert_eq!(ch.stops(), 1);
    }

    #[tokio::test]
    async fn manager_ignores_unknown_outbound_channel() {
        let (outbound_tx, outbound_rx) = broadcast::channel(16);
        let mut manager = ChannelManager::new(outbound_rx);

        let ch = Arc::new(MockChannel::new("telegram"));
        let ch_dyn: Arc<dyn Channel> = ch.clone();
        manager.register(ch_dyn).await;

        let (inbound_tx, _inbound_rx) = mpsc::channel(16);
        manager.start_all(inbound_tx).await.unwrap();
        sleep(Duration::from_millis(50)).await;

        outbound_tx
            .send(OutboundMessage {
                channel: "discord".to_string(),
                chat_id: "1".to_string(),
                content: "hello".to_string(),
                metadata: HashMap::new(),
            })
            .unwrap();
        sleep(Duration::from_millis(50)).await;

        assert_eq!(ch.sends(), 0);
        manager.stop_all().await.unwrap();
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
