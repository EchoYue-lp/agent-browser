//! # HTTP API Server for Browser Automation
//!
//! Provides RESTful API for browser control, similar to echo-browser-agent.
//!
//! ## API Endpoints
//!
//! | Endpoint | Method | Description |
//! |----------|--------|-------------|
//! | `/navigate` | POST | Navigate to URL |
//! | `/snapshot` | GET | Get Accessibility Tree |
//! | `/act` | POST | Perform element action (by ref_id) |
//! | `/screenshot` | GET | Take screenshot |
//! | `/wait` | POST | Wait for selector or timeout |
//! | `/evaluate` | POST | Execute JavaScript |
//! | `/cookies` | GET/POST | Get/Set cookies |
//! | `/tabs` | GET | List all tabs |
//! | `/tabs/{tab_id}/activate` | POST | Activate tab |
//! | `/tabs/{tab_id}` | DELETE | Close tab |
//! | `/upload` | POST | File upload |
//! | `/dialog` | POST | Register dialog handler |
//! | `/health` | GET | Health check |
//! | `/ws` | GET | WebSocket real-time events |
//!
//! ## CSS Selector-based Operations (NEW)
//!
//! These endpoints allow direct element operations using CSS selectors,
//! without needing to first get a ref_id from the snapshot:
//!
//! | Endpoint | Method | Description |
//! |----------|--------|-------------|
//! | `/click-selector` | POST | Click element by CSS selector |
//! | `/type-selector` | POST | Type text into element by CSS selector |
//! | `/get-text` | POST | Get text content of element |
//! | `/get-attribute` | POST | Get attribute value of element |
//! | `/element-exists` | POST | Check if element exists |
//! | `/hover` | POST | Hover over element by CSS selector |
//! | `/select-option` | POST | Select option in dropdown |
//! | `/submenu` | POST | Expand menu and click submenu item |

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, Query, Request, State},
    http::{StatusCode, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::info;

use agent_browser_core::{
    ActionKind, BrowserConfig, BrowserEngine, NavigationWaitUntil, PageSnapshot, ScreenshotOptions,
    SetCookieParam, SnapshotDiff, SnapshotNode, SnapshotOptions, TabInfo, actions::ActionResult,
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Global application state
pub struct AppState {
    /// Browser engine instance
    pub engine: Arc<BrowserEngine>,
    /// Configuration
    pub config: HttpConfig,
    /// Event broadcast channel for WebSocket
    pub event_tx: EventBroadcast,
    /// Explicitly isolated browser sessions.
    pub sessions: RwLock<std::collections::HashMap<String, Arc<BrowserEngine>>>,
}

pub struct SessionEngine {
    engine: Arc<BrowserEngine>,
    session_id: String,
}

#[axum::async_trait]
impl FromRequestParts<Arc<AppState>> for SessionEngine {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let session_id = parts
            .headers
            .get("X-Browser-Session")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty());
        let Some(session_id) = session_id else {
            return Ok(Self {
                engine: state.engine.clone(),
                session_id: "default".to_string(),
            });
        };
        let engine = state.sessions.read().await.get(session_id).cloned();
        engine
            .map(|engine| Self {
                engine,
                session_id: session_id.to_string(),
            })
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(ApiError {
                        error: "session_not_found".to_string(),
                        details: Some(format!("Unknown browser session: {session_id}")),
                    }),
                )
                    .into_response()
            })
    }
}

/// HTTP server configuration
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Address to bind. Defaults to loopback for safety.
    pub host: IpAddr,
    /// Server port
    pub port: u16,
    /// API key for authentication (optional)
    pub api_key: Option<String>,
    /// Default timeout in milliseconds
    pub default_timeout_ms: u64,
    /// Browser configuration
    pub browser: BrowserConfig,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 3000,
            api_key: None,
            default_timeout_ms: 30_000,
            browser: BrowserConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Error / Success wrappers
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ApiError {
    fn new(error: &str, details: impl ToString) -> Self {
        Self {
            error: error.to_string(),
            details: Some(details.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

#[derive(Debug, Serialize)]
pub struct ApiSuccess<T: Serialize> {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

fn ok<T: Serialize>(data: T) -> ApiSuccess<T> {
    ApiSuccess {
        status: "ok",
        data: Some(data),
    }
}

fn ok_empty() -> ApiSuccess<serde_json::Value> {
    ApiSuccess {
        status: "ok",
        data: None,
    }
}

fn emit_event(state: &AppState, session_id: &str, event_type: &str, data: serde_json::Value) {
    let _ = state.event_tx.send(serde_json::json!({
        "type": event_type,
        "session_id": session_id,
        "data": data,
    }));
}

fn forward_browser_events(
    engine: Arc<BrowserEngine>,
    event_tx: EventBroadcast,
    session_id: String,
) {
    let mut events = engine.subscribe_events();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let _ = event_tx.send(serde_json::json!({
                        "type": "browser_event",
                        "session_id": session_id,
                        "event": event,
                    }));
                }
                Err(broadcast::error::RecvError::Lagged(dropped)) => {
                    let _ = event_tx.send(serde_json::json!({
                        "type": "browser_event_lagged",
                        "session_id": session_id,
                        "dropped": dropped,
                    }));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn parse_wait_until(value: Option<&str>) -> Result<NavigationWaitUntil, String> {
    match value {
        None | Some("load") => Ok(NavigationWaitUntil::Load),
        Some("domContentLoaded" | "dom_content_loaded" | "DOMContentLoaded") => {
            Ok(NavigationWaitUntil::DomContentLoaded)
        }
        Some("networkIdle" | "networkidle" | "network_idle") => {
            Ok(NavigationWaitUntil::NetworkIdle)
        }
        Some("none") => Ok(NavigationWaitUntil::None),
        Some(other) => Err(format!("Unsupported wait_until value: {other}")),
    }
}

fn validate_server_config(config: &HttpConfig) -> anyhow::Result<()> {
    if !config.host.is_loopback() && config.api_key.is_none() {
        anyhow::bail!(
            "BROWSER_API_KEY is required when binding the HTTP server to a non-loopback address"
        );
    }
    Ok(())
}

impl<T: Serialize> IntoResponse for ApiSuccess<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct NavigateRequest {
    pub url: String,
    #[serde(default)]
    pub wait_until: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TabIdQuery {
    #[serde(default)]
    pub tab_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScreenshotQuery {
    #[serde(default)]
    pub full_page: Option<bool>,
    #[serde(default)]
    pub selector: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SnapshotQuery {
    #[serde(default)]
    pub interactive_only: Option<bool>,
    #[serde(default)]
    pub root_ref: Option<String>,
    #[serde(default)]
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub max_nodes: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotSearchRequest {
    pub query: String,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ActRequest {
    pub snapshot_id: String,
    pub ref_id: String,
    pub action: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub clear_first: Option<bool>,
    #[serde(default)]
    pub values: Option<Vec<String>>,
    #[serde(default)]
    pub target_ref_id: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub amount: Option<i32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct WaitRequest {
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub idle_duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct EvaluateRequest {
    pub script: String,
}

#[derive(Debug, Deserialize)]
pub struct SetCookiesRequest {
    pub cookies: Vec<SetCookieParam>,
}

#[derive(Debug, Deserialize)]
pub struct UploadRequest {
    pub snapshot_id: String,
    pub ref_id: String,
    pub file_path: String,
}

#[derive(Debug, Deserialize)]
pub struct DialogRequest {
    #[serde(default = "default_true")]
    pub accept: bool,
    #[serde(default)]
    pub prompt_text: Option<String>,
}

fn default_true() -> bool {
    true
}

// CSS Selector-based request types

#[derive(Debug, Deserialize)]
pub struct SelectorRequest {
    /// CSS selector
    pub selector: String,
    /// Timeout in milliseconds
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct TypeSelectorRequest {
    /// CSS selector
    pub selector: String,
    /// Text to type
    pub text: String,
    /// Clear existing text first
    #[serde(default)]
    pub clear_first: Option<bool>,
    /// Timeout in milliseconds
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GetAttributeRequest {
    /// CSS selector
    pub selector: String,
    /// Attribute name
    pub attribute: String,
    /// Timeout in milliseconds
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct SelectOptionRequest {
    /// CSS selector for the select element
    pub selector: String,
    /// Value or text to select
    pub value: String,
    /// Select by visible text instead of value
    #[serde(default)]
    pub by_text: Option<bool>,
    /// Timeout in milliseconds
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct SubmenuRequest {
    /// CSS selector for the main menu item
    pub menu_selector: String,
    /// CSS selector for the submenu item
    pub submenu_selector: String,
    /// Timeout in milliseconds
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct NavigateResponse {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub snapshot_id: String,
    pub url: String,
    pub title: String,
    pub nodes: Vec<SnapshotNode>,
    pub iframe_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub iframe_mappings: Vec<IframeMappingInfo>,
}

#[derive(Debug, Serialize)]
pub struct IframeMappingInfo {
    pub ref_id: String,
    pub frame_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScreenshotResponse {
    pub image: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub action: ActionResult,
    pub snapshot: PageSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<SnapshotDiff>,
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(ref required_key) = state.config.api_key {
        let x_api_key = request
            .headers()
            .get("X-API-Key")
            .and_then(|v| v.to_str().ok());
        let bearer = request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if x_api_key != Some(required_key.as_str()) && bearer != Some(required_key.as_str()) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "unauthorized".to_string(),
                    details: Some("Missing or invalid X-API-Key header".to_string()),
                }),
            )
                .into_response();
        }
    }
    next.run(request).await
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub browser_running: bool,
    pub responsive: bool,
}

/// POST /sessions - Create an isolated browser session.
pub async fn create_session(State(state): State<Arc<AppState>>) -> Json<ApiSuccess<SessionInfo>> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let engine = Arc::new(BrowserEngine::new(state.config.browser.clone()));
    forward_browser_events(engine.clone(), state.event_tx.clone(), session_id.clone());
    state
        .sessions
        .write()
        .await
        .insert(session_id.clone(), engine);
    Json(ok(SessionInfo {
        session_id,
        browser_running: false,
        responsive: true,
    }))
}

/// GET /sessions - List explicit browser sessions.
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Json<ApiSuccess<Vec<SessionInfo>>> {
    let sessions = state
        .sessions
        .read()
        .await
        .iter()
        .map(|(id, engine)| (id.clone(), engine.clone()))
        .collect::<Vec<_>>();
    let mut result = Vec::with_capacity(sessions.len());
    for (session_id, engine) in sessions {
        let browser_running = engine.is_launched().await;
        let responsive = !browser_running || engine.health_check().await;
        result.push(SessionInfo {
            session_id,
            browser_running,
            responsive,
        });
    }
    Json(ok(result))
}

/// DELETE /sessions/{session_id} - Close and remove a browser session.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let engine = state
        .sessions
        .write()
        .await
        .remove(&session_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new(
                    "session_not_found",
                    format!("Unknown browser session: {session_id}"),
                )),
            )
        })?;
    engine.shutdown().await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("session_shutdown_failed", error)),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /navigate
pub async fn navigate(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
    Json(req): Json<NavigateRequest>,
) -> Result<Json<ApiSuccess<NavigateResponse>>, (StatusCode, Json<ApiError>)> {
    info!("Navigate: {}", req.url);

    let engine = engine.as_ref();

    let wait_until = parse_wait_until(req.wait_until.as_deref()).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError::new("invalid_wait_until", error)),
        )
    })?;

    let result = engine
        .navigate_with_options(&req.url, wait_until)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiError::new("navigation_failed", e)),
            )
        })?;

    emit_event(
        &state,
        &session_id,
        "navigation",
        serde_json::json!({
            "url": &result.final_url,
            "title": &result.title,
            "wait_until": &req.wait_until,
        }),
    );

    Ok(Json(ok(NavigateResponse {
        url: result.final_url,
        title: result.title,
    })))
}

/// GET /snapshot
pub async fn snapshot(
    SessionEngine { engine, .. }: SessionEngine,
    Query(req): Query<SnapshotQuery>,
) -> Result<Json<ApiSuccess<SnapshotResponse>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let defaults = SnapshotOptions::default();
    let snap = engine
        .snapshot_with_options(SnapshotOptions {
            interactive_only: req.interactive_only.unwrap_or(defaults.interactive_only),
            root_ref: req.root_ref,
            max_depth: req.max_depth.or(defaults.max_depth),
            max_nodes: req.max_nodes.unwrap_or(defaults.max_nodes).clamp(1, 5_000),
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("snapshot_failed", e)),
            )
        })?;

    let iframe_mappings: Vec<IframeMappingInfo> = snap
        .iframe_mappings
        .iter()
        .map(|m| IframeMappingInfo {
            ref_id: m.ref_id.clone(),
            frame_id: m.frame_id.clone(),
            name: m.name.clone(),
            src: m.src.clone(),
        })
        .collect();

    Ok(Json(ok(SnapshotResponse {
        snapshot_id: snap.snapshot_id,
        url: snap.url,
        title: snap.title,
        nodes: snap.nodes,
        iframe_count: snap.iframe_count,
        iframe_mappings,
    })))
}

/// POST /snapshot/search - Search the latest snapshot without returning the full tree.
pub async fn search_snapshot(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SnapshotSearchRequest>,
) -> Result<Json<ApiSuccess<agent_browser_core::SnapshotSearchResult>>, (StatusCode, Json<ApiError>)>
{
    let result = engine
        .search_snapshot(&req.query, req.max_results.unwrap_or(20).clamp(1, 200))
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new("snapshot_search_failed", error)),
            )
        })?;
    Ok(Json(ok(result)))
}

/// POST /act - Perform action on element
pub async fn act(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
    Json(req): Json<ActRequest>,
) -> Result<Json<ApiSuccess<ActionResponse>>, (StatusCode, Json<ApiError>)> {
    info!("Act: {} on {}", req.action, req.ref_id);

    let engine = engine.as_ref();

    let action = match req.action.as_str() {
        "click" => ActionKind::Click,
        "double_click" => ActionKind::DoubleClick,
        "right_click" => ActionKind::RightClick,
        "hover" => ActionKind::Hover,
        "focus" => ActionKind::Focus,
        "type" => {
            let text = req.text.clone().ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiError::new("missing_param", "'text' required for type")),
                )
            })?;
            ActionKind::Type {
                text,
                clear_first: req.clear_first,
            }
        }
        "press" => {
            let key = req.key.clone().ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiError::new("missing_param", "'key' required for press")),
                )
            })?;
            ActionKind::Press { key }
        }
        "select" => {
            let values = req.values.clone().ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiError::new(
                        "missing_param",
                        "'values' required for select",
                    )),
                )
            })?;
            ActionKind::Select { values }
        }
        "drag" => {
            let target = req.target_ref_id.clone().ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiError::new(
                        "missing_param",
                        "'target_ref_id' required for drag",
                    )),
                )
            })?;
            ActionKind::Drag {
                target_ref_id: target,
            }
        }
        "scroll" => ActionKind::Scroll {
            direction: Some(req.direction.clone().unwrap_or_else(|| "down".to_string())),
            amount: Some(req.amount.unwrap_or(300)),
        },
        "wait" => ActionKind::Wait {
            timeout_ms: Some(req.timeout_ms.unwrap_or(1000)),
        },
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError::new(
                    "unknown_action",
                    format!("Unknown action: {}", other),
                )),
            ));
        }
    };

    let action_result = engine
        .act_with_snapshot(&req.snapshot_id, &req.ref_id, action)
        .await
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiError::new("action_failed", e)),
            )
        })?;
    emit_event(
        &state,
        &session_id,
        "action",
        serde_json::json!({
            "action": req.action,
            "ref_id": req.ref_id,
            "success": action_result.success,
        }),
    );
    let snapshot = engine
        .snapshot_with_options(SnapshotOptions::default())
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("post_action_snapshot_failed", error)),
            )
        })?;
    let diff = engine.latest_snapshot_diff().await;
    Ok(Json(ok(ActionResponse {
        action: action_result,
        snapshot,
        diff,
    })))
}

/// GET /screenshot
pub async fn screenshot(
    SessionEngine { engine, .. }: SessionEngine,
    Query(req): Query<ScreenshotQuery>,
) -> Result<Json<ApiSuccess<ScreenshotResponse>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let options = ScreenshotOptions {
        full_page: req.full_page,
        selector: req.selector,
    };

    let result = engine.screenshot_with_options(options).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("screenshot_failed", e)),
        )
    })?;

    Ok(Json(ok(ScreenshotResponse {
        image: result.data,
        format: result.format,
        width: result.width,
        height: result.height,
    })))
}

/// POST /wait
pub async fn wait(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<WaitRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let timeout = req.timeout_ms.unwrap_or(state.config.default_timeout_ms);

    let engine = engine.as_ref();

    if let Some(ref selector) = req.selector {
        engine
            .wait_for_selector(selector, timeout)
            .await
            .map_err(|e| {
                (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(ApiError::new("wait_timeout", e)),
                )
            })?;
        Ok(Json(ok(serde_json::json!({ "selector": selector }))))
    } else if let Some(idle_ms) = req.idle_duration_ms {
        engine
            .wait_for_network_idle(idle_ms, timeout)
            .await
            .map_err(|e| {
                (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(ApiError::new("wait_timeout", e)),
                )
            })?;
        Ok(Json(ok(serde_json::json!({ "waited": "network_idle" }))))
    } else {
        engine.wait(timeout).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("wait_failed", e)),
            )
        })?;
        Ok(Json(ok_empty()))
    }
}

/// POST /evaluate
pub async fn evaluate(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<EvaluateRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let value = engine.evaluate(&req.script).await.map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ApiError::new("evaluate_failed", e)),
        )
    })?;

    Ok(Json(ok(value)))
}

/// GET /cookies
pub async fn get_cookies(
    SessionEngine { engine, .. }: SessionEngine,
) -> Result<Json<ApiSuccess<Vec<agent_browser_core::CookieInfo>>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let cookies = engine.get_cookies().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("cookies_failed", e)),
        )
    })?;

    Ok(Json(ok(cookies)))
}

/// POST /cookies
pub async fn set_cookies(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SetCookiesRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    engine.set_cookies(req.cookies).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("set_cookies_failed", e)),
        )
    })?;

    Ok(Json(ok_empty()))
}

/// GET /tabs
pub async fn list_tabs(
    SessionEngine { engine, .. }: SessionEngine,
) -> Result<Json<ApiSuccess<Vec<TabInfo>>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let tabs = engine.list_tabs().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("tabs_failed", e)),
        )
    })?;

    Ok(Json(ok(tabs)))
}

/// POST /tabs/{tab_id}/activate
pub async fn activate_tab(
    SessionEngine { engine, .. }: SessionEngine,
    Path(tab_id): Path<String>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    engine.activate_tab(&tab_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError::new("tab_not_found", e)),
        )
    })?;

    Ok(Json(ok(serde_json::json!({ "tab_id": tab_id }))))
}

/// DELETE /tabs/{tab_id}
pub async fn close_tab(
    SessionEngine { engine, .. }: SessionEngine,
    Path(tab_id): Path<String>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    engine.close_tab(&tab_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError::new("tab_not_found", e)),
        )
    })?;

    info!("Closed tab: {}", tab_id);
    Ok(Json(ok_empty()))
}

/// POST /upload
pub async fn upload(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
    Json(req): Json<UploadRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    info!("Upload: {} -> ref_id={}", req.file_path, req.ref_id);

    let engine = engine.as_ref();

    engine
        .upload_file_with_snapshot(&req.snapshot_id, &req.ref_id, &req.file_path)
        .await
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiError::new("upload_failed", e)),
            )
        })?;

    emit_event(
        &state,
        &session_id,
        "upload",
        serde_json::json!({"file": &req.file_path, "ref_id": &req.ref_id}),
    );

    Ok(Json(ok(serde_json::json!({ "file": req.file_path }))))
}

/// POST /dialog
pub async fn dialog(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<DialogRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    engine
        .setup_dialog_handler(req.accept, req.prompt_text)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("dialog_setup_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({ "accept": req.accept }))))
}

/// POST /shutdown
pub async fn shutdown(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    engine.shutdown().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("shutdown_failed", e)),
        )
    })?;

    emit_event(&state, &session_id, "shutdown", serde_json::json!({}));

    Ok(Json(ok_empty()))
}

// ---------------------------------------------------------------------------
// iframe 上下文
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EnterIframeRequest {
    pub snapshot_id: String,
    pub ref_id: String,
}

/// POST /iframe/enter
pub async fn enter_iframe(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
    Json(req): Json<EnterIframeRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let depth = engine
        .enter_iframe_with_snapshot(&req.snapshot_id, &req.ref_id)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("iframe_not_found", e)),
            )
        })?;

    emit_event(
        &state,
        &session_id,
        "iframe_entered",
        serde_json::json!({"depth": depth, "ref_id": &req.ref_id}),
    );

    Ok(Json(ok(
        serde_json::json!({ "depth": depth, "ref_id": req.ref_id }),
    )))
}

/// POST /iframe/exit
pub async fn exit_iframe(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let depth = engine.exit_iframe().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("exit_iframe_failed", e)),
        )
    })?;

    emit_event(
        &state,
        &session_id,
        "iframe_exited",
        serde_json::json!({"depth": depth}),
    );

    Ok(Json(ok(serde_json::json!({ "depth": depth }))))
}

/// POST /iframe/exit-all
pub async fn exit_all_iframes(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    engine.exit_all_iframes().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("exit_all_iframes_failed", e)),
        )
    })?;

    emit_event(
        &state,
        &session_id,
        "iframe_exited_all",
        serde_json::json!({"depth": 0}),
    );

    Ok(Json(ok(serde_json::json!({ "depth": 0 }))))
}

// ---------------------------------------------------------------------------
// 文件下载
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DownloadFileRequest {
    pub url: String,
    #[serde(default)]
    pub save_path: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ClickDownloadRequest {
    pub snapshot_id: String,
    pub ref_id: String,
    #[serde(default)]
    pub save_path: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// POST /download
pub async fn download_file(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
    Json(req): Json<DownloadFileRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let options = agent_browser_core::DownloadOptions {
        save_path: req.save_path,
        timeout_ms: req.timeout_ms,
    };

    let result = engine
        .download_file(&req.url, Some(options))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("download_failed", e)),
            )
        })?;

    emit_event(
        &state,
        &session_id,
        "download",
        serde_json::json!({
            "guid": &result.guid,
            "filename": &result.filename,
            "file_path": &result.file_path,
            "size": result.size,
        }),
    );

    Ok(Json(ok(serde_json::json!({
        "guid": result.guid,
        "filename": result.filename,
        "file_path": result.file_path,
        "size": result.size,
        "status": result.status
    }))))
}

/// POST /click-download
pub async fn click_and_download(
    State(state): State<Arc<AppState>>,
    SessionEngine { engine, session_id }: SessionEngine,
    Json(req): Json<ClickDownloadRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let options = agent_browser_core::DownloadOptions {
        save_path: req.save_path,
        timeout_ms: req.timeout_ms,
    };

    let result = engine
        .click_and_download_with_snapshot(&req.snapshot_id, &req.ref_id, Some(options))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("download_failed", e)),
            )
        })?;

    emit_event(
        &state,
        &session_id,
        "download",
        serde_json::json!({
            "guid": &result.guid,
            "filename": &result.filename,
            "file_path": &result.file_path,
            "size": result.size,
        }),
    );

    Ok(Json(ok(serde_json::json!({
        "guid": result.guid,
        "filename": result.filename,
        "file_path": result.file_path,
        "size": result.size,
        "status": result.status
    }))))
}

// ---------------------------------------------------------------------------
// 键盘快捷键
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PressKeyRequest {
    pub key: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShortcutRequest {
    pub shortcut: String,
}

/// POST /press-key
pub async fn press_key(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<PressKeyRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let modifiers: Vec<agent_browser_core::KeyModifier> = req
        .modifiers
        .iter()
        .filter_map(|s| match s.to_lowercase().as_str() {
            "alt" => Some(agent_browser_core::KeyModifier::Alt),
            "control" | "ctrl" => Some(agent_browser_core::KeyModifier::Control),
            "meta" | "cmd" | "command" => Some(agent_browser_core::KeyModifier::Meta),
            "shift" => Some(agent_browser_core::KeyModifier::Shift),
            _ => None,
        })
        .collect();

    let result = engine
        .press_with_modifiers(&req.key, &modifiers)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("press_key_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({
        "success": result.success,
        "message": result.message
    }))))
}

/// POST /shortcut
pub async fn send_shortcut(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<ShortcutRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let result = engine.send_shortcut(&req.shortcut).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError::new("shortcut_failed", e)),
        )
    })?;

    Ok(Json(ok(serde_json::json!({
        "success": result.success,
        "message": result.message
    }))))
}

// ---------------------------------------------------------------------------
// CSS Selector-based operations
// ---------------------------------------------------------------------------

/// POST /click-selector
/// Click an element by CSS selector directly (without needing ref_id)
pub async fn click_selector(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let result = engine
        .click_selector(&req.selector, req.timeout_ms)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("click_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({
        "success": result.success,
        "message": result.message
    }))))
}

/// POST /type-selector
/// Type text into an element by CSS selector
pub async fn type_selector(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<TypeSelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let result = engine
        .type_selector(
            &req.selector,
            &req.text,
            req.clear_first.unwrap_or(false),
            req.timeout_ms,
        )
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, Json(ApiError::new("type_failed", e))))?;

    Ok(Json(ok(serde_json::json!({
        "success": result.success,
        "message": result.message
    }))))
}

/// POST /get-text
/// Get text content of an element by CSS selector
pub async fn get_text(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let text = engine
        .get_text(&req.selector, req.timeout_ms)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("get_text_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({
        "selector": req.selector,
        "text": text
    }))))
}

/// POST /get-attribute
/// Get an attribute value of an element by CSS selector
pub async fn get_attribute(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<GetAttributeRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let value = engine
        .get_attribute(&req.selector, &req.attribute, req.timeout_ms)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("get_attribute_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({
        "selector": req.selector,
        "attribute": req.attribute,
        "value": value
    }))))
}

/// POST /element-exists
/// Check if an element exists by CSS selector
pub async fn element_exists(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let exists = engine.element_exists(&req.selector).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("check_failed", e)),
        )
    })?;

    Ok(Json(ok(serde_json::json!({
        "selector": req.selector,
        "exists": exists
    }))))
}

/// POST /hover
/// Hover over an element by CSS selector
pub async fn hover_selector(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let result = engine
        .hover_selector(&req.selector, req.timeout_ms)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("hover_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({
        "success": result.success,
        "message": result.message
    }))))
}

/// POST /select-option
/// Select an option in a select element by CSS selector
pub async fn select_option(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SelectOptionRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let result = engine
        .select_option(
            &req.selector,
            &req.value,
            req.by_text.unwrap_or(false),
            req.timeout_ms,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("select_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({
        "success": result.success,
        "message": result.message
    }))))
}

/// POST /submenu
/// Expand a menu and click a submenu item
pub async fn expand_and_click_submenu(
    SessionEngine { engine, .. }: SessionEngine,
    Json(req): Json<SubmenuRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = engine.as_ref();

    let result = engine
        .expand_and_click_submenu(&req.menu_selector, &req.submenu_selector, req.timeout_ms)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("submenu_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({
        "success": result.success,
        "message": result.message
    }))))
}

/// GET /health
pub async fn health(SessionEngine { engine, .. }: SessionEngine) -> impl IntoResponse {
    let launched = engine.is_launched().await;
    let responsive = !launched || engine.health_check().await;
    let status = if responsive {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(ok(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "status": if !responsive { "unhealthy" } else if launched { "ready" } else { "idle" },
            "browser_running": launched && responsive,
        }))),
    )
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

/// Broadcast channel capacity
const WS_CHANNEL_CAP: usize = 256;

/// Event broadcast sender
pub type EventBroadcast = Arc<broadcast::Sender<serde_json::Value>>;

/// GET /ws - WebSocket endpoint
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    SessionEngine { session_id, .. }: SessionEngine,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state, session_id))
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>, session_id: String) {
    let mut rx = state.event_tx.subscribe();
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Keepalive interval
    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(15));
    keepalive.tick().await; // skip first

    loop {
        tokio::select! {
            // Forward events to WebSocket
            ev = rx.recv() => {
                match ev {
                    Ok(event) => {
                        if !event_belongs_to_session(&event, &session_id) {
                            continue;
                        }
                        let msg = serde_json::to_string(&event).unwrap_or_default();
                        if ws_tx.send(Message::Text(msg)).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let _ = ws_tx.send(Message::Text(
                            serde_json::json!({"type":"lagged","dropped":n}).to_string()
                        )).await;
                    }
                    Err(_) => break,
                }
            }

            // Process client messages
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text)
                            && let Some("pong") = cmd["type"].as_str()
                        {}
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }

            // Keepalive ping
            _ = keepalive.tick() => {
                let msg = serde_json::json!({"type":"ping"}).to_string();
                if ws_tx.send(Message::Text(msg)).await.is_err() { break; }
            }
        }
    }
}

fn event_belongs_to_session(event: &serde_json::Value, session_id: &str) -> bool {
    event.get("session_id").and_then(serde_json::Value::as_str) == Some(session_id)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sessions", post(create_session).get(list_sessions))
        .route("/sessions/{session_id}", delete(delete_session))
        .route("/navigate", post(navigate))
        .route("/snapshot", get(snapshot))
        .route("/snapshot/search", post(search_snapshot))
        .route("/act", post(act))
        .route("/screenshot", get(screenshot))
        .route("/wait", post(wait))
        .route("/evaluate", post(evaluate))
        .route("/upload", post(upload))
        .route("/dialog", post(dialog))
        .route("/cookies", get(get_cookies).post(set_cookies))
        .route("/tabs", get(list_tabs))
        .route("/tabs/{tab_id}/activate", post(activate_tab))
        .route("/tabs/{tab_id}", delete(close_tab))
        .route("/iframe/enter", post(enter_iframe))
        .route("/iframe/exit", post(exit_iframe))
        .route("/iframe/exit-all", post(exit_all_iframes))
        .route("/download", post(download_file))
        .route("/click-download", post(click_and_download))
        .route("/press-key", post(press_key))
        .route("/shortcut", post(send_shortcut))
        // CSS Selector-based operations
        .route("/click-selector", post(click_selector))
        .route("/type-selector", post(type_selector))
        .route("/get-text", post(get_text))
        .route("/get-attribute", post(get_attribute))
        .route("/element-exists", post(element_exists))
        .route("/hover", post(hover_selector))
        .route("/select-option", post(select_option))
        .route("/submenu", post(expand_and_click_submenu))
        .route("/shutdown", post(shutdown))
        .route("/health", get(health))
        .route("/ws", get(ws_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

pub async fn run_server(config: HttpConfig) -> anyhow::Result<()> {
    validate_server_config(&config)?;

    // Initialize browser engine
    let engine = BrowserEngine::new(config.browser.clone());

    // Create event broadcast
    let (event_tx, _) = broadcast::channel(WS_CHANNEL_CAP);
    let event_tx = Arc::new(event_tx);

    // Create app state
    let state = Arc::new(AppState {
        engine: Arc::new(engine),
        config: config.clone(),
        event_tx,
        sessions: RwLock::new(std::collections::HashMap::new()),
    });
    forward_browser_events(
        state.engine.clone(),
        state.event_tx.clone(),
        "default".to_string(),
    );

    // Build router
    let app = build_router(state);

    // Start server
    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP server listening on http://{}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

/// Parse configuration from environment
pub fn config_from_env() -> HttpConfig {
    let mut config = HttpConfig {
        browser: BrowserConfig::from_env(),
        ..HttpConfig::default()
    };

    if let Ok(host) = std::env::var("BROWSER_HTTP_HOST")
        && let Ok(host) = host.parse()
    {
        config.host = host;
    }

    if let Ok(port) = std::env::var("BROWSER_HTTP_PORT") {
        config.port = port.parse().unwrap_or(3000);
    }

    if let Ok(api_key) = std::env::var("BROWSER_API_KEY")
        && !api_key.is_empty()
    {
        config.api_key = Some(api_key);
    }

    if let Ok(timeout) = std::env::var("BROWSER_DEFAULT_TIMEOUT_MS") {
        config.default_timeout_ms = timeout.parse().unwrap_or(30_000);
    }

    config
}

fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Get configuration
    let config = config_from_env();

    // Run server
    tokio::runtime::Runtime::new()?.block_on(run_server(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_defaults_to_loopback() {
        let config = HttpConfig::default();
        assert!(config.host.is_loopback());
        assert!(validate_server_config(&config).is_ok());
    }

    #[test]
    fn non_loopback_requires_api_key() {
        let mut config = HttpConfig {
            host: "0.0.0.0".parse().unwrap(),
            ..HttpConfig::default()
        };
        assert!(validate_server_config(&config).is_err());
        config.api_key = Some("secret".to_string());
        assert!(validate_server_config(&config).is_ok());
    }

    #[test]
    fn parses_navigation_wait_strategies() {
        assert_eq!(parse_wait_until(None).unwrap(), NavigationWaitUntil::Load);
        assert_eq!(
            parse_wait_until(Some("domContentLoaded")).unwrap(),
            NavigationWaitUntil::DomContentLoaded
        );
        assert_eq!(
            parse_wait_until(Some("networkidle")).unwrap(),
            NavigationWaitUntil::NetworkIdle
        );
        assert_eq!(
            parse_wait_until(Some("none")).unwrap(),
            NavigationWaitUntil::None
        );
        assert!(parse_wait_until(Some("networkidle0")).is_err());
    }

    #[tokio::test]
    async fn creates_lists_and_deletes_isolated_sessions() {
        let config = HttpConfig::default();
        let (event_tx, _) = broadcast::channel(16);
        let state = Arc::new(AppState {
            engine: Arc::new(BrowserEngine::new(config.browser.clone())),
            config,
            event_tx: Arc::new(event_tx),
            sessions: RwLock::new(std::collections::HashMap::new()),
        });

        let created = create_session(State(state.clone())).await.0.data.unwrap();
        let listed = list_sessions(State(state.clone())).await.0.data.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, created.session_id);

        let status = delete_session(State(state.clone()), Path(created.session_id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(state.sessions.read().await.is_empty());
    }

    #[test]
    fn websocket_events_are_session_scoped() {
        let event = serde_json::json!({"session_id": "session-a", "type": "action"});
        assert!(event_belongs_to_session(&event, "session-a"));
        assert!(!event_belongs_to_session(&event, "session-b"));
        assert!(!event_belongs_to_session(
            &serde_json::json!({"type": "action"}),
            "session-a"
        ));
    }
}
