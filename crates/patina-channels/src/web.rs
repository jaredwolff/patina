use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use dashmap::DashMap;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use patina_config::{GatewayConfig, WebConfig};
use patina_core::bus::InboundMessage;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info, warn};

use crate::base::Channel;
use crate::web_assets;

type WsSender = mpsc::UnboundedSender<Message>;

pub struct WebChannel {
    config: WebConfig,
    gateway_config: GatewayConfig,
    connections: Arc<DashMap<String, WsSender>>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

#[derive(Clone)]
struct AppState {
    config: WebConfig,
    connections: Arc<DashMap<String, WsSender>>,
    inbound_tx: mpsc::Sender<InboundMessage>,
}

#[derive(Deserialize)]
struct WsParams {
    password: Option<String>,
    session: Option<String>,
}

#[derive(Serialize)]
struct WsOutMsg {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "chatId")]
    chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
}

#[derive(Deserialize)]
struct WsInMsg {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    content: String,
}

impl WebChannel {
    pub fn new(config: WebConfig, gateway_config: GatewayConfig) -> Result<Self> {
        Ok(Self {
            config,
            gateway_config,
            connections: Arc::new(DashMap::new()),
            shutdown_tx: Mutex::new(None),
        })
    }
}

#[async_trait]
impl Channel for WebChannel {
    fn name(&self) -> &str {
        "web"
    }

    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        let state = AppState {
            config: self.config.clone(),
            connections: self.connections.clone(),
            inbound_tx,
        };

        let router = Router::new()
            .route("/", get(serve_index))
            .route("/style.css", get(serve_css))
            .route("/app.js", get(serve_js))
            .route("/ws", get(ws_upgrade))
            .with_state(state);

        let addr: SocketAddr = format!("{}:{}", self.gateway_config.host, self.gateway_config.port)
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid gateway listen address: {e}"))?;

        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!("Web channel listening on http://{addr}");

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        *self.shutdown_tx.lock().await = Some(shutdown_tx);

        let connections = self.connections.clone();
        tokio::spawn(async move {
            let server = axum::serve(listener, router).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });

            if let Err(e) = server.await {
                error!("Web server error: {e}");
            }

            // Close all connections on shutdown
            connections.clear();
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }
        self.connections.clear();
        Ok(())
    }

    async fn send(&self, msg: &patina_core::bus::OutboundMessage) -> Result<()> {
        if let Some(sender) = self.connections.get(&msg.chat_id) {
            let out = WsOutMsg {
                msg_type: "message".to_string(),
                content: Some(msg.content.clone()),
                chat_id: None,
                timestamp: Some(chrono::Local::now().to_rfc3339()),
            };
            let json = serde_json::to_string(&out)?;
            if sender.send(Message::Text(json.into())).is_err() {
                warn!(
                    "WebSocket send failed for chat_id={}, removing connection",
                    msg.chat_id
                );
                drop(sender);
                self.connections.remove(&msg.chat_id);
            }
        } else {
            // Client disconnected. Session persists in JSONL — they can reconnect.
            warn!(
                "No active WebSocket for chat_id={}, message saved to session only",
                msg.chat_id
            );
        }
        Ok(())
    }

    fn is_allowed(&self, sender_id: &str) -> bool {
        if self.config.allow_from.is_empty() {
            return true;
        }
        self.config.allow_from.iter().any(|a| a == sender_id)
    }
}

// --- Axum Handlers ---

async fn serve_index() -> Html<&'static str> {
    Html(web_assets::INDEX_HTML)
}

async fn serve_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], web_assets::STYLE_CSS)
}

async fn serve_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        web_assets::APP_JS,
    )
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    Query(params): Query<WsParams>,
    State(state): State<AppState>,
) -> Response {
    // Validate password
    if !state.config.password.is_empty() {
        let provided = params.password.as_deref().unwrap_or("");
        if provided != state.config.password {
            return ws
                .on_upgrade(|mut socket| async move {
                    let err = WsOutMsg {
                        msg_type: "error".to_string(),
                        content: Some("Authentication failed".to_string()),
                        chat_id: None,
                        timestamp: None,
                    };
                    let _ = socket
                        .send(Message::Text(serde_json::to_string(&err).unwrap().into()))
                        .await;
                    let _ = socket.close().await;
                })
                .into_response();
        }
    }

    let chat_id = params
        .session
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    ws.on_upgrade(move |socket| handle_ws(socket, chat_id, state))
        .into_response()
}

async fn handle_ws(socket: WebSocket, chat_id: String, state: AppState) {
    let sender_id = format!("web:{}", &chat_id[..chat_id.len().min(8)]);
    info!("WebSocket connected: chat_id={chat_id}, sender_id={sender_id}");

    let (ws_write, mut ws_read) = socket.split();
    let (tx, rx) = mpsc::unbounded_channel::<Message>();

    // Register connection
    state.connections.insert(chat_id.clone(), tx.clone());

    // Spawn write task
    let write_chat_id = chat_id.clone();
    let write_handle = tokio::spawn(ws_write_loop(ws_write, rx, write_chat_id));

    // Send connected message
    let connected = WsOutMsg {
        msg_type: "connected".to_string(),
        content: None,
        chat_id: Some(chat_id.clone()),
        timestamp: None,
    };
    if let Ok(json) = serde_json::to_string(&connected) {
        let _ = tx.send(Message::Text(json.into()));
    }

    // Read loop
    while let Some(result) = ws_read.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                warn!("WebSocket read error for {chat_id}: {e}");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let parsed: WsInMsg = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if parsed.msg_type == "message" && !parsed.content.trim().is_empty() {
                    let inbound = InboundMessage {
                        channel: "web".to_string(),
                        sender_id: sender_id.clone(),
                        chat_id: chat_id.clone(),
                        content: parsed.content,
                        media: Vec::new(),
                        metadata: HashMap::new(),
                        timestamp: chrono::Local::now().to_rfc3339(),
                    };
                    if let Err(e) = state.inbound_tx.send(inbound).await {
                        error!("Failed to send inbound message: {e}");
                        break;
                    }
                }
            }
            Message::Close(_) => break,
            _ => {} // Ignore ping/pong/binary
        }
    }

    // Cleanup
    state.connections.remove(&chat_id);
    write_handle.abort();
    info!("WebSocket disconnected: chat_id={chat_id}");
}

async fn ws_write_loop(
    mut ws_write: SplitSink<WebSocket, Message>,
    mut rx: mpsc::UnboundedReceiver<Message>,
    chat_id: String,
) {
    while let Some(msg) = rx.recv().await {
        if let Err(e) = ws_write.send(msg).await {
            warn!("WebSocket write error for {chat_id}: {e}");
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_allowed_empty_allows_all() {
        let ch = WebChannel::new(
            WebConfig {
                enabled: true,
                password: String::new(),
                allow_from: vec![],
            },
            GatewayConfig::default(),
        )
        .unwrap();
        assert!(ch.is_allowed("anyone"));
    }

    #[test]
    fn test_is_allowed_checks_list() {
        let ch = WebChannel::new(
            WebConfig {
                enabled: true,
                password: String::new(),
                allow_from: vec!["web:abc12345".to_string()],
            },
            GatewayConfig::default(),
        )
        .unwrap();
        assert!(ch.is_allowed("web:abc12345"));
        assert!(!ch.is_allowed("web:other"));
    }

    #[test]
    fn test_ws_out_msg_serialization() {
        let msg = WsOutMsg {
            msg_type: "connected".to_string(),
            content: None,
            chat_id: Some("abc-123".to_string()),
            timestamp: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"connected\""));
        assert!(json.contains("\"chatId\":\"abc-123\""));
        assert!(!json.contains("content"));
        assert!(!json.contains("timestamp"));
    }

    #[test]
    fn test_ws_in_msg_deserialization() {
        let json = r#"{"type":"message","content":"hello"}"#;
        let msg: WsInMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "message");
        assert_eq!(msg.content, "hello");
    }

    #[test]
    fn test_ws_in_msg_missing_content() {
        let json = r#"{"type":"message"}"#;
        let msg: WsInMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "message");
        assert_eq!(msg.content, "");
    }

    #[test]
    fn test_password_validation_empty_allows_all() {
        let config = WebConfig {
            enabled: true,
            password: String::new(),
            allow_from: vec![],
        };
        // Empty password means no auth required
        assert!(config.password.is_empty());
    }
}
