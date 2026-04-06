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
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use agent_browser_core::{
    BrowserConfig, BrowserEngine, HeadlessMode, ScreenshotOptions, SetCookieParam, SnapshotNode,
    TabInfo, actions::ActionResult,
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
}

/// HTTP server configuration
#[derive(Debug, Clone)]
pub struct HttpConfig {
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

#[derive(Debug, Deserialize)]
pub struct ActRequest {
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

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(ref required_key) = state.config.api_key {
        let provided = request
            .headers()
            .get("X-API-Key")
            .and_then(|v| v.to_str().ok());
        if provided != Some(required_key.as_str()) {
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

/// POST /navigate
pub async fn navigate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<NavigateRequest>,
) -> Result<Json<ApiSuccess<NavigateResponse>>, (StatusCode, Json<ApiError>)> {
    info!("Navigate: {}", req.url);

    let engine = &state.engine;

    let result = engine.navigate(&req.url).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiError::new("navigation_failed", e)),
        )
    })?;

    // Apply wait_until
    if let Some(ref wait) = req.wait_until
        && wait.as_str() == "networkidle"
    {
        let _ = engine.wait_for_network_idle(500, 30_000).await;
    }

    Ok(Json(ok(NavigateResponse {
        url: req.url,
        title: result.title,
    })))
}

/// GET /snapshot
pub async fn snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiSuccess<SnapshotResponse>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

    let snap = engine.snapshot().await.map_err(|e| {
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
        url: snap.url,
        title: snap.title,
        nodes: snap.nodes,
        iframe_count: snap.iframe_count,
        iframe_mappings,
    })))
}

/// POST /act - Perform action on element
pub async fn act(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ActRequest>,
) -> Result<Json<ApiSuccess<ActionResult>>, (StatusCode, Json<ApiError>)> {
    info!("Act: {} on {}", req.action, req.ref_id);

    let engine = &state.engine;

    let result = match req.action.as_str() {
        "click" => engine.click(&req.ref_id).await,
        "double_click" => engine.double_click(&req.ref_id).await,
        "right_click" => engine.right_click(&req.ref_id).await,
        "hover" => engine.hover(&req.ref_id).await,
        "focus" => engine.focus(&req.ref_id).await,
        "type" => {
            let text = req.text.clone().ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiError::new("missing_param", "'text' required for type")),
                )
            })?;
            engine
                .type_text(&req.ref_id, &text, req.clear_first.unwrap_or(false))
                .await
        }
        "press" => {
            let key = req.key.clone().ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiError::new("missing_param", "'key' required for press")),
                )
            })?;
            engine.press(&req.ref_id, &key).await
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
            engine.select(&req.ref_id, values).await
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
            engine.drag(&req.ref_id, &target).await
        }
        "scroll" => {
            let direction = req.direction.as_deref().unwrap_or("down");
            engine.scroll(direction, req.amount.unwrap_or(300)).await
        }
        "wait" => {
            let timeout = req.timeout_ms.unwrap_or(1000);
            engine.wait(timeout).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiError::new("wait_failed", e)),
                )
            })?;
            return Ok(Json(ok(ActionResult {
                success: true,
                message: format!("Waited {}ms", timeout),
            })));
        }
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

    match result {
        Ok(action_result) => Ok(Json(ok(action_result))),
        Err(e) => Ok(Json(ok(ActionResult {
            success: false,
            message: e.to_string(),
        }))),
    }
}

/// GET /screenshot
pub async fn screenshot(
    State(state): State<Arc<AppState>>,
    Query(req): Query<ScreenshotQuery>,
) -> Result<Json<ApiSuccess<ScreenshotResponse>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    Json(req): Json<WaitRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let timeout = req.timeout_ms.unwrap_or(state.config.default_timeout_ms);

    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<EvaluateRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiSuccess<Vec<agent_browser_core::CookieInfo>>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetCookiesRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiSuccess<Vec<TabInfo>>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Path(tab_id): Path<String>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Path(tab_id): Path<String>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    Json(req): Json<UploadRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    info!("Upload: {} -> ref_id={}", req.file_path, req.ref_id);

    let engine = &state.engine;

    engine
        .upload_file(&req.ref_id, &req.file_path)
        .await
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiError::new("upload_failed", e)),
            )
        })?;

    Ok(Json(ok(serde_json::json!({ "file": req.file_path }))))
}

/// POST /dialog
pub async fn dialog(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DialogRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

    engine.shutdown().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("shutdown_failed", e)),
        )
    })?;

    Ok(Json(ok_empty()))
}

// ---------------------------------------------------------------------------
// iframe 上下文
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EnterIframeRequest {
    pub ref_id: String,
}

/// POST /iframe/enter
pub async fn enter_iframe(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EnterIframeRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

    let depth = engine.enter_iframe(&req.ref_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError::new("iframe_not_found", e)),
        )
    })?;

    Ok(Json(ok(
        serde_json::json!({ "depth": depth, "ref_id": req.ref_id }),
    )))
}

/// POST /iframe/exit
pub async fn exit_iframe(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

    let depth = engine.exit_iframe().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("exit_iframe_failed", e)),
        )
    })?;

    Ok(Json(ok(serde_json::json!({ "depth": depth }))))
}

/// POST /iframe/exit-all
pub async fn exit_all_iframes(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

    engine.exit_all_iframes().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new("exit_all_iframes_failed", e)),
        )
    })?;

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
    pub ref_id: String,
    #[serde(default)]
    pub save_path: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// POST /download
pub async fn download_file(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DownloadFileRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    Json(req): Json<ClickDownloadRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

    let options = agent_browser_core::DownloadOptions {
        save_path: req.save_path,
        timeout_ms: req.timeout_ms,
    };

    let result = engine
        .click_and_download(&req.ref_id, Some(options))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new("download_failed", e)),
            )
        })?;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<PressKeyRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<ShortcutRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<TypeSelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<GetAttributeRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<SelectorRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<SelectOptionRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmenuRequest>,
) -> Result<Json<ApiSuccess<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let engine = &state.engine;

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
pub async fn health() -> Json<ApiSuccess<serde_json::Value>> {
    Json(ok(
        serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "status": "ok" }),
    ))
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
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
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

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/navigate", post(navigate))
        .route("/snapshot", get(snapshot))
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
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

pub async fn run_server(config: HttpConfig) -> anyhow::Result<()> {
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
    });

    // Build router
    let app = build_router(state);

    // Start server
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP server listening on http://{}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

/// Parse configuration from environment
pub fn config_from_env() -> HttpConfig {
    let mut config = HttpConfig::default();

    if let Ok(port) = std::env::var("BROWSER_HTTP_PORT") {
        config.port = port.parse().unwrap_or(3000);
    }

    if let Ok(api_key) = std::env::var("BROWSER_API_KEY") {
        config.api_key = Some(api_key);
    }

    if let Ok(timeout) = std::env::var("BROWSER_DEFAULT_TIMEOUT_MS") {
        config.default_timeout_ms = timeout.parse().unwrap_or(30_000);
    }

    if std::env::var("BROWSER_HEADLESS").is_ok() {
        config.browser.headless = HeadlessMode::New;
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
