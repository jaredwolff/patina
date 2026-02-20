use std::collections::HashMap;
use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path as AxumPath, Query, State, WebSocketUpgrade};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, put};
use axum::Router;
use dashmap::DashMap;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use patina_config::schema::ModelPricing;
use patina_config::{GatewayConfig, WebConfig};
use patina_core::agent::ModelPool;
use patina_core::bus::InboundMessage;
use patina_core::persona::PersonaStore;
use patina_core::usage::{UsageFilter, UsageTracker};
use rig::completion::{CompletionModel, CompletionRequest, Message as RigMessage};
use rig::message::{AssistantContent, Text, UserContent};
use rig::OneOrMany;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info, warn};

use crate::base::Channel;
use crate::web_assets;

type WsSender = mpsc::UnboundedSender<Message>;

pub struct WebChannel {
    config: WebConfig,
    gateway_config: GatewayConfig,
    sessions_dir: PathBuf,
    connections: Arc<DashMap<String, WsSender>>,
    persona_store: Arc<tokio::sync::Mutex<PersonaStore>>,
    models: ModelPool,
    usage_tracker: Option<Arc<UsageTracker>>,
    pricing: HashMap<String, ModelPricing>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

#[derive(Clone)]
struct AppState {
    config: WebConfig,
    sessions_dir: PathBuf,
    connections: Arc<DashMap<String, WsSender>>,
    inbound_tx: mpsc::Sender<InboundMessage>,
    persona_store: Arc<tokio::sync::Mutex<PersonaStore>>,
    models: ModelPool,
    model_tiers: Vec<String>,
    usage_tracker: Option<Arc<UsageTracker>>,
    pricing: HashMap<String, ModelPricing>,
}

#[derive(Deserialize)]
struct WsParams {
    password: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    messages: Option<Vec<HistoryMessage>>,
}

#[derive(Serialize, Clone)]
struct HistoryMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
}

#[derive(Deserialize)]
struct WsInMsg {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    #[serde(rename = "chatId")]
    chat_id: String,
    #[serde(default)]
    persona: String,
}

impl WebChannel {
    pub fn new(
        config: WebConfig,
        gateway_config: GatewayConfig,
        sessions_dir: PathBuf,
        persona_store: Arc<tokio::sync::Mutex<PersonaStore>>,
        models: ModelPool,
        usage_tracker: Option<Arc<UsageTracker>>,
        pricing: HashMap<String, ModelPricing>,
    ) -> Result<Self> {
        Ok(Self {
            config,
            gateway_config,
            sessions_dir,
            connections: Arc::new(DashMap::new()),
            persona_store,
            models,
            usage_tracker,
            pricing,
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
        let model_tiers = self.models.tiers().iter().map(|s| s.to_string()).collect();
        let state = AppState {
            config: self.config.clone(),
            sessions_dir: self.sessions_dir.clone(),
            connections: self.connections.clone(),
            inbound_tx,
            persona_store: self.persona_store.clone(),
            models: self.models.clone(),
            model_tiers,
            usage_tracker: self.usage_tracker.clone(),
            pricing: self.pricing.clone(),
        };

        let router = Router::new()
            .route("/", get(serve_index))
            .route("/style.css", get(serve_css))
            .route("/app.js", get(serve_js))
            .route("/marked.min.js", get(serve_marked_js))
            .route("/ws", get(ws_upgrade))
            .route("/api/sessions", get(api_list_sessions))
            .route(
                "/api/sessions/{id}",
                axum::routing::delete(api_delete_session),
            )
            .route(
                "/api/personas",
                get(api_list_personas).post(api_create_persona),
            )
            .route(
                "/api/personas/generate-prompt",
                axum::routing::post(api_generate_prompt),
            )
            .route(
                "/api/personas/{key}",
                put(api_update_persona).delete(api_delete_persona),
            )
            .route("/api/model-tiers", get(api_model_tiers))
            .route("/api/usage/summary", get(api_usage_summary))
            .route("/api/usage/daily", get(api_usage_daily))
            .route("/api/usage/filters", get(api_usage_filters))
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
        if self.connections.is_empty() {
            warn!(
                "No active WebSocket connections, message for chat_id={} saved to session only",
                msg.chat_id
            );
            return Ok(());
        }

        let out = WsOutMsg {
            msg_type: "message".to_string(),
            content: Some(msg.content.clone()),
            chat_id: Some(msg.chat_id.clone()),
            timestamp: Some(chrono::Local::now().to_rfc3339()),
            messages: None,
        };
        let json = serde_json::to_string(&out)?;

        // Broadcast to all connections — client filters by chatId
        for entry in self.connections.iter() {
            if entry
                .value()
                .send(Message::Text(json.clone().into()))
                .is_err()
            {
                warn!(
                    "WebSocket send failed for conn={}, will clean up on disconnect",
                    entry.key()
                );
            }
        }
        Ok(())
    }

    fn is_allowed(&self, sender_id: &str) -> bool {
        if self.config.allow_from.is_empty() {
            return true;
        }
        self.config.allow_from.iter().any(|a| a == sender_id)
    }

    fn prompt_rules(&self) -> &str {
        self.config.system_prompt_rules.as_deref().unwrap_or("")
    }
}

impl WebChannel {
    /// Broadcast a streaming text chunk to all connected WebSocket clients.
    pub fn broadcast_chunk(&self, chat_id: &str, text: &str) {
        let out = WsOutMsg {
            msg_type: "text_delta".to_string(),
            content: Some(text.to_string()),
            chat_id: Some(chat_id.to_string()),
            timestamp: None,
            messages: None,
        };
        if let Ok(json) = serde_json::to_string(&out) {
            for entry in self.connections.iter() {
                let _ = entry.value().send(Message::Text(json.clone().into()));
            }
        }
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

async fn serve_marked_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        web_assets::MARKED_JS,
    )
}

async fn api_list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let sessions = list_web_sessions(&state.sessions_dir);
    axum::Json(sessions)
}

async fn api_delete_session(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    // Reject path traversal
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "invalid session id"})),
        );
    }
    let filename = format!("web_{id}.jsonl");
    let path = state.sessions_dir.join(&filename);
    if !path.exists() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "session not found"})),
        );
    }
    match std::fs::remove_file(&path) {
        Ok(()) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({"deleted": true})),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

// --- Persona API ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PersonaResponse {
    key: String,
    name: String,
    description: String,
    preamble: String,
    model_tier: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    color: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersonaRequest {
    #[serde(default)]
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    preamble: String,
    #[serde(default)]
    model_tier: String,
    #[serde(default)]
    color: String,
}

async fn api_list_personas(State(state): State<AppState>) -> impl IntoResponse {
    let store = state.persona_store.lock().await;
    let personas: Vec<PersonaResponse> = store
        .list()
        .iter()
        .map(|(k, p)| PersonaResponse {
            key: k.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            preamble: p.preamble.clone(),
            model_tier: p.model_tier.clone(),
            color: p.color.clone(),
        })
        .collect();
    axum::Json(personas)
}

async fn api_create_persona(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<PersonaRequest>,
) -> impl IntoResponse {
    if req.key.is_empty() || req.name.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "key and name are required"})),
        );
    }
    let mut store = state.persona_store.lock().await;
    let persona = patina_core::persona::Persona {
        name: req.name,
        description: req.description,
        preamble: req.preamble,
        model_tier: req.model_tier,
        color: req.color,
    };
    match store.upsert(req.key.clone(), persona) {
        Ok(()) => (
            axum::http::StatusCode::CREATED,
            axum::Json(serde_json::json!({"key": req.key})),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn api_update_persona(
    State(state): State<AppState>,
    AxumPath(key): AxumPath<String>,
    axum::Json(req): axum::Json<PersonaRequest>,
) -> impl IntoResponse {
    if req.name.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "name is required"})),
        );
    }
    let mut store = state.persona_store.lock().await;
    let persona = patina_core::persona::Persona {
        name: req.name,
        description: req.description,
        preamble: req.preamble,
        model_tier: req.model_tier,
        color: req.color,
    };
    match store.upsert(key.clone(), persona) {
        Ok(()) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({"key": key})),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn api_delete_persona(
    State(state): State<AppState>,
    AxumPath(key): AxumPath<String>,
) -> impl IntoResponse {
    let mut store = state.persona_store.lock().await;
    match store.remove(&key) {
        Ok(true) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({"deleted": true})),
        ),
        Ok(false) => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "persona not found"})),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn api_model_tiers(State(state): State<AppState>) -> impl IntoResponse {
    axum::Json(state.model_tiers.clone())
}

// --- Usage API ---

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageQueryParams {
    from: Option<String>,
    to: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    agent: Option<String>,
    session: Option<String>,
    group_by: Option<String>,
}

impl UsageQueryParams {
    fn to_filter(&self) -> UsageFilter {
        UsageFilter {
            from: self.from.clone(),
            to: self.to.clone(),
            model: self.model.clone(),
            provider: self.provider.clone(),
            agent: self.agent.clone(),
            session: self.session.clone(),
            group_by: self.group_by.clone(),
        }
    }
}

async fn api_usage_summary(
    State(state): State<AppState>,
    Query(params): Query<UsageQueryParams>,
) -> impl IntoResponse {
    let tracker = match &state.usage_tracker {
        Some(t) => t,
        None => {
            return axum::Json(serde_json::json!([]));
        }
    };
    match tracker.query_summary_with_cost(&params.to_filter(), &state.pricing) {
        Ok(rows) => axum::Json(serde_json::to_value(rows).unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn api_usage_daily(
    State(state): State<AppState>,
    Query(params): Query<UsageQueryParams>,
) -> impl IntoResponse {
    let tracker = match &state.usage_tracker {
        Some(t) => t,
        None => {
            return axum::Json(serde_json::json!([]));
        }
    };
    match tracker.query_daily_with_cost(&params.to_filter(), &state.pricing) {
        Ok(rows) => axum::Json(serde_json::to_value(rows).unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn api_usage_filters(State(state): State<AppState>) -> impl IntoResponse {
    let tracker = match &state.usage_tracker {
        Some(t) => t,
        None => {
            return axum::Json(serde_json::json!({
                "models": [],
                "providers": [],
                "agents": [],
            }));
        }
    };
    axum::Json(serde_json::json!({
        "models": tracker.distinct_values("model").unwrap_or_default(),
        "providers": tracker.distinct_values("provider").unwrap_or_default(),
        "agents": tracker.distinct_values("agent").unwrap_or_default(),
    }))
}

#[derive(Deserialize)]
struct GeneratePromptRequest {
    name: String,
    #[serde(default)]
    description: String,
}

async fn api_generate_prompt(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<GeneratePromptRequest>,
) -> impl IntoResponse {
    if req.name.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "name is required"})),
        );
    }

    let (model, _model_name, _provider_name) = state.models.default_model();

    let prompt = format!(
        "Generate a system prompt for an AI assistant persona with the following details:\n\
         Name: {}\n\
         Description: {}\n\n\
         Write a concise, effective system prompt that defines the persona's behavior, \
         tone, expertise, and boundaries. Return ONLY the system prompt text, nothing else.",
        req.name,
        if req.description.is_empty() {
            "(no description provided)"
        } else {
            &req.description
        }
    );

    let request = CompletionRequest {
        preamble: None,
        chat_history: OneOrMany::one(RigMessage::User {
            content: OneOrMany::one(UserContent::Text(Text { text: prompt })),
        }),
        documents: Vec::new(),
        tools: Vec::new(),
        temperature: Some(0.7),
        max_tokens: Some(1024),
        tool_choice: None,
        additional_params: None,
    };

    match model.completion(request).await {
        Ok(response) => {
            let text: String = response
                .choice
                .iter()
                .filter_map(|c| match c {
                    AssistantContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect();
            (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({"preamble": text.trim()})),
            )
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
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
                        messages: None,
                    };
                    let _ = socket
                        .send(Message::Text(serde_json::to_string(&err).unwrap().into()))
                        .await;
                    let _ = socket.close().await;
                })
                .into_response();
        }
    }

    ws.on_upgrade(move |socket| handle_ws(socket, state))
        .into_response()
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let conn_id = uuid::Uuid::new_v4().to_string();
    let short_conn = &conn_id[..8];
    info!("WebSocket connected: conn={short_conn}");

    let (ws_write, mut ws_read) = socket.split();
    let (tx, rx) = mpsc::unbounded_channel::<Message>();

    state.connections.insert(conn_id.clone(), tx.clone());

    let write_conn_id = conn_id.clone();
    let write_handle = tokio::spawn(ws_write_loop(ws_write, rx, write_conn_id));

    // Send connected acknowledgment
    let connected = WsOutMsg {
        msg_type: "connected".to_string(),
        content: None,
        chat_id: None,
        timestamp: None,
        messages: None,
    };
    if let Ok(json) = serde_json::to_string(&connected) {
        let _ = tx.send(Message::Text(json.into()));
    }

    // Read loop
    while let Some(result) = ws_read.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                warn!("WebSocket read error for conn={short_conn}: {e}");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let parsed: WsInMsg = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                match parsed.msg_type.as_str() {
                    "get_history" => {
                        if parsed.chat_id.is_empty() {
                            continue;
                        }
                        send_history(&state.sessions_dir, &parsed.chat_id, &tx);
                    }
                    "message" => {
                        let chat_id = parsed.chat_id;
                        if chat_id.is_empty() || parsed.content.trim().is_empty() {
                            continue;
                        }

                        // Echo user message to other connected clients
                        broadcast_to_others(
                            &state.connections,
                            &conn_id,
                            &WsOutMsg {
                                msg_type: "user_message".to_string(),
                                content: Some(parsed.content.clone()),
                                chat_id: Some(chat_id.clone()),
                                timestamp: Some(chrono::Local::now().to_rfc3339()),
                                messages: None,
                            },
                        );

                        // Notify other clients that agent is processing
                        broadcast_to_others(
                            &state.connections,
                            &conn_id,
                            &WsOutMsg {
                                msg_type: "thinking".to_string(),
                                content: None,
                                chat_id: Some(chat_id.clone()),
                                timestamp: None,
                                messages: None,
                            },
                        );

                        let sender_id = format!("web:{}", &chat_id[..chat_id.len().min(8)]);

                        let mut metadata = HashMap::new();
                        if !parsed.persona.is_empty() {
                            metadata.insert(
                                "persona".to_string(),
                                serde_json::Value::String(parsed.persona.clone()),
                            );
                        }

                        let inbound = InboundMessage {
                            channel: "web".to_string(),
                            sender_id,
                            chat_id,
                            content: parsed.content,
                            media: Vec::new(),
                            metadata,
                            timestamp: chrono::Local::now().to_rfc3339(),
                        };
                        if let Err(e) = state.inbound_tx.send(inbound).await {
                            error!("Failed to send inbound message: {e}");
                            break;
                        }
                    }
                    "create_session" => {
                        if parsed.chat_id.is_empty() {
                            continue;
                        }
                        broadcast_to_others(
                            &state.connections,
                            &conn_id,
                            &WsOutMsg {
                                msg_type: "session_created".to_string(),
                                content: Some(parsed.content.clone()),
                                chat_id: Some(parsed.chat_id.clone()),
                                timestamp: Some(chrono::Local::now().to_rfc3339()),
                                messages: None,
                            },
                        );
                    }
                    "delete_session" => {
                        if parsed.chat_id.is_empty() {
                            continue;
                        }
                        broadcast_to_others(
                            &state.connections,
                            &conn_id,
                            &WsOutMsg {
                                msg_type: "session_deleted".to_string(),
                                content: None,
                                chat_id: Some(parsed.chat_id.clone()),
                                timestamp: None,
                                messages: None,
                            },
                        );
                    }
                    _ => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Cleanup
    state.connections.remove(&conn_id);
    write_handle.abort();
    info!("WebSocket disconnected: conn={short_conn}");
}

/// Broadcast a message to all WebSocket connections except the sender.
fn broadcast_to_others(
    connections: &DashMap<String, WsSender>,
    exclude_conn_id: &str,
    msg: &WsOutMsg,
) {
    if let Ok(json) = serde_json::to_string(msg) {
        for entry in connections.iter() {
            if entry.key() != exclude_conn_id {
                let _ = entry.value().send(Message::Text(json.clone().into()));
            }
        }
    }
}

/// Send session history for a chat over a WS connection.
fn send_history(
    sessions_dir: &std::path::Path,
    chat_id: &str,
    tx: &mpsc::UnboundedSender<Message>,
) {
    let history = load_session_history(sessions_dir, chat_id);
    if !history.is_empty() {
        let history_msg = WsOutMsg {
            msg_type: "history".to_string(),
            content: None,
            chat_id: Some(chat_id.to_string()),
            timestamp: None,
            messages: Some(history),
        };
        if let Ok(json) = serde_json::to_string(&history_msg) {
            let _ = tx.send(Message::Text(json.into()));
        }
    }
}

async fn ws_write_loop(
    mut ws_write: SplitSink<WebSocket, Message>,
    mut rx: mpsc::UnboundedReceiver<Message>,
    conn_id: String,
) {
    while let Some(msg) = rx.recv().await {
        if let Err(e) = ws_write.send(msg).await {
            warn!("WebSocket write error for conn={conn_id}: {e}");
            break;
        }
    }
}

// --- Session History ---

const MAX_HISTORY_MESSAGES: usize = 100;

/// Load message history from a session's JSONL file.
/// Returns user/assistant messages only, capped at MAX_HISTORY_MESSAGES.
fn load_session_history(sessions_dir: &std::path::Path, chat_id: &str) -> Vec<HistoryMessage> {
    // Mirror SessionManager::session_path: replace ':' with '_'
    let session_key = format!("web:{chat_id}");
    let safe_key = session_key.replace(':', "_");
    let path = sessions_dir.join(format!("{safe_key}.jsonl"));

    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = std::io::BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Skip metadata lines
        if value.get("_type").is_some() {
            continue;
        }

        let role = match value.get("role").and_then(|r| r.as_str()) {
            Some(r) if r == "user" || r == "assistant" => r.to_string(),
            _ => continue,
        };

        let content = match value.get("content").and_then(|c| c.as_str()) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => continue,
        };

        let timestamp = value
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(|t| t.to_string());

        messages.push(HistoryMessage {
            role,
            content,
            timestamp,
        });
    }

    // Return only the last N messages
    if messages.len() > MAX_HISTORY_MESSAGES {
        messages.split_off(messages.len() - MAX_HISTORY_MESSAGES)
    } else {
        messages
    }
}

/// Session info returned by the sessions listing API.
#[derive(Serialize)]
struct SessionInfo {
    id: String,
    title: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    persona: Option<String>,
}

/// List web channel sessions from the sessions directory.
fn list_web_sessions(sessions_dir: &std::path::Path) -> Vec<SessionInfo> {
    let entries = match std::fs::read_dir(sessions_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut sessions = Vec::new();

    for entry in entries.flatten() {
        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.starts_with("web_") || !filename.ends_with(".jsonl") {
            continue;
        }

        // Extract chat_id: "web_{uuid}.jsonl" -> "{uuid}"
        let chat_id = filename
            .strip_prefix("web_")
            .and_then(|s| s.strip_suffix(".jsonl"))
            .unwrap_or("")
            .to_string();
        if chat_id.is_empty() {
            continue;
        }

        let file = match std::fs::File::open(entry.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = std::io::BufReader::new(file);

        let mut updated_at = String::new();
        let mut title = String::new();
        let mut persona = None;

        for line in reader.lines().flatten() {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Read metadata for updated_at and persona
            if value.get("_type").and_then(|t| t.as_str()) == Some("metadata") {
                if let Some(ts) = value.get("updated_at").and_then(|t| t.as_str()) {
                    updated_at = ts.to_string();
                }
                if let Some(p) = value
                    .get("metadata")
                    .and_then(|m| m.get("persona"))
                    .and_then(|p| p.as_str())
                {
                    if !p.is_empty() {
                        persona = Some(p.to_string());
                    }
                }
                continue;
            }

            // Find first user message for title
            if title.is_empty() {
                if value.get("role").and_then(|r| r.as_str()) == Some("user") {
                    if let Some(content) = value.get("content").and_then(|c| c.as_str()) {
                        title = content.chars().take(50).collect();
                        if content.len() > 50 {
                            title.push_str("...");
                        }
                    }
                }
            }

            // Once we have both, stop reading
            if !updated_at.is_empty() && !title.is_empty() {
                break;
            }
        }

        if title.is_empty() {
            title = "New Chat".to_string();
        }

        sessions.push(SessionInfo {
            id: chat_id,
            title,
            updated_at,
            persona,
        });
    }

    // Sort by updated_at descending
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use tempfile::TempDir;

    fn test_sessions_dir() -> PathBuf {
        PathBuf::from("/tmp/patina-test-sessions")
    }

    #[allow(deprecated)]
    fn test_model_pool() -> ModelPool {
        use rig::client::completion::CompletionModelHandle;
        use rig::client::{CompletionClient, Nothing};
        use rig::providers::ollama;

        let client: ollama::Client = ollama::Client::builder().api_key(Nothing).build().unwrap();
        let handle = CompletionModelHandle::new(Arc::new(client.completion_model("test")));
        let mut models = HashMap::new();
        models.insert(
            "default".to_string(),
            (handle, "test".to_string(), "ollama".to_string()),
        );
        ModelPool::new(models)
    }

    fn test_persona_store() -> Arc<tokio::sync::Mutex<PersonaStore>> {
        Arc::new(tokio::sync::Mutex::new(PersonaStore::load(&PathBuf::from(
            "/tmp/patina-test-personas.json",
        ))))
    }

    #[test]
    fn test_is_allowed_empty_allows_all() {
        let ch = WebChannel::new(
            WebConfig {
                enabled: true,
                password: String::new(),
                allow_from: vec![],
                system_prompt_rules: None,
            },
            GatewayConfig::default(),
            test_sessions_dir(),
            test_persona_store(),
            test_model_pool(),
            None,
            HashMap::new(),
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
                system_prompt_rules: None,
            },
            GatewayConfig::default(),
            test_sessions_dir(),
            test_persona_store(),
            test_model_pool(),
            None,
            HashMap::new(),
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
            messages: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"connected\""));
        assert!(json.contains("\"chatId\":\"abc-123\""));
        assert!(!json.contains("content"));
        assert!(!json.contains("timestamp"));
        assert!(!json.contains("messages"));
    }

    #[test]
    fn test_ws_out_msg_with_history() {
        let msg = WsOutMsg {
            msg_type: "history".to_string(),
            content: None,
            chat_id: None,
            timestamp: None,
            messages: Some(vec![
                HistoryMessage {
                    role: "user".to_string(),
                    content: "hello".to_string(),
                    timestamp: Some("2026-01-01T00:00:00".to_string()),
                },
                HistoryMessage {
                    role: "assistant".to_string(),
                    content: "hi there".to_string(),
                    timestamp: None,
                },
            ]),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"history\""));
        assert!(json.contains("\"messages\""));
        assert!(json.contains("\"hello\""));
        assert!(json.contains("\"hi there\""));
    }

    #[test]
    fn test_ws_in_msg_deserialization() {
        let json = r#"{"type":"message","content":"hello"}"#;
        let msg: WsInMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "message");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.chat_id, "");
    }

    #[test]
    fn test_ws_in_msg_with_chat_id() {
        let json = r#"{"type":"message","content":"hello","chatId":"abc-123"}"#;
        let msg: WsInMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "message");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.chat_id, "abc-123");
    }

    #[test]
    fn test_ws_in_msg_get_history() {
        let json = r#"{"type":"get_history","chatId":"abc-123"}"#;
        let msg: WsInMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "get_history");
        assert_eq!(msg.chat_id, "abc-123");
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
            system_prompt_rules: None,
        };
        assert!(config.password.is_empty());
    }

    #[test]
    fn test_load_session_history_missing_file() {
        let dir = TempDir::new().unwrap();
        let history = load_session_history(dir.path(), "nonexistent-uuid");
        assert!(history.is_empty());
    }

    #[test]
    fn test_load_session_history_parses_jsonl() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("web_test-uuid.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"_type":"metadata","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"role":"user","content":"hello","timestamp":"2026-01-01T00:00:01Z"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"role":"assistant","content":"hi there","timestamp":"2026-01-01T00:00:02Z"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"role":"system","content":"system msg","timestamp":"2026-01-01T00:00:03Z"}}"#
        )
        .unwrap();
        drop(f);

        let history = load_session_history(dir.path(), "test-uuid");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "hi there");
    }

    #[test]
    fn test_load_session_history_caps_at_max() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("web_big-uuid.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"_type":"metadata"}}"#).unwrap();
        for i in 0..150 {
            writeln!(f, r#"{{"role":"user","content":"msg {i}"}}"#).unwrap();
        }
        drop(f);

        let history = load_session_history(dir.path(), "big-uuid");
        assert_eq!(history.len(), MAX_HISTORY_MESSAGES);
        // Should be the last 100
        assert!(history[0].content.contains("50"));
    }

    #[test]
    fn test_list_web_sessions() {
        let dir = TempDir::new().unwrap();

        // Create a web session file
        let path = dir.path().join("web_abc-123.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"_type":"metadata","updated_at":"2026-01-01T12:00:00Z"}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"role":"user","content":"What is Rust?"}}"#).unwrap();
        drop(f);

        // Create a non-web session file (should be ignored)
        let other = dir.path().join("telegram_999.jsonl");
        std::fs::write(&other, r#"{"_type":"metadata"}"#).unwrap();

        let sessions = list_web_sessions(dir.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "abc-123");
        assert_eq!(sessions[0].title, "What is Rust?");
        assert_eq!(sessions[0].updated_at, "2026-01-01T12:00:00Z");
    }
}
