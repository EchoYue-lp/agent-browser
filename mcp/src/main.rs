//! Echo Browser MCP Server
//!
//! 基于 MCP 2025-11-25 协议的浏览器控制服务器。
//!
//! ## 支持的传输
//!
//! - **stdio**: 标准输入/输出（默认）
//! - **sse**: Server-Sent Events (计划中)
//! - **http**: Streamable HTTP (计划中)
//!
//! ## 使用方式
//!
//! ### Claude Code 配置
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "browser": {
//!       "command": "/path/to/agent-browser-mcp"
//!     }
//!   }
//! }
//! ```
//!
//! ### 命令行选项
//!
//! ```text
//! agent-browser-mcp [OPTIONS]
//!
//! Options:
//!   --transport <TYPE>  Transport type: stdio (default)
//!   --port <PORT>       Port for HTTP/SSE transport (default: 3000)
//!   --help              Show help
//! ```

use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use agent_browser_core::snapshot::format_snapshot;
use agent_browser_core::{BrowserConfig, BrowserEngine, ScreenshotOptions, SetCookieParam};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};
use tracing::{info, warn};

mod protocol;
mod tools;
mod transport;

use protocol::*;
use tools::*;

// ---------------------------------------------------------------------------
// MCP Server 状态
// ---------------------------------------------------------------------------

/// MCP Server 状态
struct ServerState {
    browser: BrowserEngine,
    initialized: std::sync::atomic::AtomicBool,
    notification_tx: mpsc::UnboundedSender<JsonRpcNotification>,
    tasks: Mutex<HashMap<String, Arc<TaskEntry>>>,
}

struct TaskEntry {
    runtime: Mutex<TaskRuntime>,
    notify: Notify,
}

struct TaskRuntime {
    descriptor: TaskDescriptor,
    result: Option<Value>,
    abort_handle: Option<tokio::task::AbortHandle>,
    expires_at: Option<tokio::time::Instant>,
}

impl ServerState {
    fn new(notification_tx: mpsc::UnboundedSender<JsonRpcNotification>) -> Self {
        Self {
            browser: BrowserEngine::new(BrowserConfig::from_env()),
            initialized: std::sync::atomic::AtomicBool::new(false),
            notification_tx,
            tasks: Mutex::new(HashMap::new()),
        }
    }

    fn is_initialized(&self) -> bool {
        self.initialized.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set_initialized(&self) {
        self.initialized
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn send_progress(&self, token: &Value, progress: f64, total: f64, message: &str) {
        let _ = self.notification_tx.send(JsonRpcNotification::new(
            "notifications/progress",
            Some(json!({
                "progressToken": token,
                "progress": progress,
                "total": total,
                "message": message,
            })),
        ));
    }

    async fn get_task(&self, task_id: &str) -> Option<Arc<TaskEntry>> {
        let entry = self.tasks.lock().await.get(task_id).cloned()?;
        let expired = entry
            .runtime
            .lock()
            .await
            .expires_at
            .is_some_and(|expires_at| expires_at <= tokio::time::Instant::now());
        if expired {
            self.tasks.lock().await.remove(task_id);
            None
        } else {
            Some(entry)
        }
    }

    async fn prune_tasks(&self) {
        let entries = self
            .tasks
            .lock()
            .await
            .iter()
            .map(|(task_id, entry)| (task_id.clone(), entry.clone()))
            .collect::<Vec<_>>();
        let now = tokio::time::Instant::now();
        let mut expired = Vec::new();
        for (task_id, entry) in entries {
            if entry
                .runtime
                .lock()
                .await
                .expires_at
                .is_some_and(|expires_at| expires_at <= now)
            {
                expired.push(task_id);
            }
        }
        let mut tasks = self.tasks.lock().await;
        for task_id in expired {
            tasks.remove(&task_id);
        }
    }
}

// ---------------------------------------------------------------------------
// 主入口
// ---------------------------------------------------------------------------

/// 传输类型
#[derive(Debug, Clone, Copy, Default)]
enum TransportType {
    #[default]
    Stdio,
    #[allow(dead_code)]
    Sse,
    #[allow(dead_code)]
    Http,
}

impl std::str::FromStr for TransportType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" => Ok(TransportType::Stdio),
            "sse" => Ok(TransportType::Sse),
            "http" => Ok(TransportType::Http),
            _ => Err(format!("Unknown transport type: {}", s)),
        }
    }
}

fn parse_args() -> (TransportType, u16) {
    let args: Vec<String> = std::env::args().collect();
    let mut transport = TransportType::default();
    let mut port: u16 = 3000;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--transport" | "-t" => {
                let value = args.get(i + 1).unwrap_or_else(|| {
                    eprintln!("Error: --transport requires a value");
                    std::process::exit(1);
                });
                transport = value.parse().unwrap_or_else(|error| {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                });
                i += 2;
                continue;
            }
            "--port" | "-p" => {
                let value = args.get(i + 1).unwrap_or_else(|| {
                    eprintln!("Error: --port requires a value");
                    std::process::exit(1);
                });
                port = value.parse().unwrap_or_else(|_| {
                    eprintln!("Error: Invalid port number");
                    std::process::exit(1);
                });
                i += 2;
                continue;
            }
            "--help" | "-h" => {
                println!("agent-browser-mcp [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --transport <TYPE>  Transport type: stdio (default), sse, http");
                println!("  --port <PORT>       Port for HTTP/SSE transport (default: 3000)");
                println!("  --help              Show this help");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    (transport, port)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let (transport, _port) = parse_args();

    match transport {
        TransportType::Stdio => run_stdio_server().await,
        TransportType::Sse => {
            info!("SSE transport not yet implemented, falling back to stdio");
            run_stdio_server().await
        }
        TransportType::Http => {
            info!("HTTP transport not yet implemented, falling back to stdio");
            run_stdio_server().await
        }
    }
}

// ---------------------------------------------------------------------------
// STDIO Server
// ---------------------------------------------------------------------------

/// 运行 STDIO MCP Server
async fn run_stdio_server() -> anyhow::Result<()> {
    info!(
        "Starting Agent Browser MCP Server v{} (protocol: {}, transport: stdio)",
        env!("CARGO_PKG_VERSION"),
        MCP_PROTOCOL_VERSION
    );

    let transport = transport::stdio::StdioTransport::new();
    let (notification_tx, mut notification_rx) = mpsc::unbounded_channel();
    let state = Arc::new(ServerState::new(notification_tx));

    let notification_transport = transport.clone();
    tokio::spawn(async move {
        while let Some(notification) = notification_rx.recv().await {
            if let Err(error) = notification_transport
                .write_notification(&notification)
                .await
            {
                warn!("Failed to send MCP notification: {error}");
                break;
            }
        }
    });

    let notification_state = state.clone();
    transport
        .run(
            move |request| {
                let state = state.clone();
                async move { handle_request(state, request).await }
            },
            move |notification| handle_notification(&notification_state, &notification),
        )
        .await
}

/// 处理 MCP 请求
async fn handle_request(state: Arc<ServerState>, request: JsonRpcRequest) -> JsonRpcResponse {
    let id = request.id.clone();

    if !matches!(request.method.as_str(), "initialize" | "ping") && !state.is_initialized() {
        return JsonRpcResponse::error_response(
            id,
            ERR_INVALID_REQUEST,
            "Server has not received notifications/initialized",
        );
    }

    match request.method.as_str() {
        // ── 生命周期 ────────────────────────────────────────────────────────
        "initialize" => handle_initialize(id, request.params),

        "ping" => JsonRpcResponse::success(id, json!({})),

        // ── 工具 ─────────────────────────────────────────────────────────────
        "tools/list" => handle_tools_list(id),

        "tools/call" => handle_tools_call(&state, id, request.params).await,

        // ── Durable tasks ───────────────────────────────────────────────────
        "tasks/get" => handle_task_get(&state, id, request.params).await,

        "tasks/list" => handle_tasks_list(&state, id).await,

        "tasks/result" => handle_task_result(&state, id, request.params).await,

        "tasks/cancel" => handle_task_cancel(&state, id, request.params).await,

        // ── 资源 ─────────────────────────────────────────────────────────────
        "resources/list" => handle_resources_list(id),

        "resources/read" => handle_resources_read(&state, id, request.params).await,

        // ── 提示词 ───────────────────────────────────────────────────────────
        "prompts/list" => handle_prompts_list(id),

        "prompts/get" => handle_prompts_get(id, request.params),

        // ── 日志 ─────────────────────────────────────────────────────────────
        "logging/setLevel" => handle_set_log_level(id, request.params),

        // ── 未知方法 ─────────────────────────────────────────────────────────
        method => {
            warn!("Unknown method: {}", method);
            JsonRpcResponse::error_response(
                id,
                ERR_METHOD_NOT_FOUND,
                &format!("Method not found: {}", method),
            )
        }
    }
}

/// 处理 MCP 通知
fn handle_notification(state: &ServerState, notification: &JsonRpcNotification) {
    match notification.method.as_str() {
        "notifications/initialized" => {
            info!("Client initialized");
            state.set_initialized();
        }
        "notifications/cancelled" => {
            info!("Request cancelled by client");
        }
        method => {
            info!("Received notification: {}", method);
        }
    }
}

// ---------------------------------------------------------------------------
// 请求处理器
// ---------------------------------------------------------------------------

/// 处理 initialize 请求
fn handle_initialize(id: Option<Value>, params: Option<Value>) -> JsonRpcResponse {
    let init = match params.and_then(|value| serde_json::from_value::<InitializeParams>(value).ok())
    {
        Some(init) => init,
        None => {
            return JsonRpcResponse::error_response(
                id,
                ERR_INVALID_PARAMS,
                "Missing or invalid initialize params",
            );
        }
    };
    info!(
        "Client '{}' v{} requested protocol version {}",
        init.client_info.name, init.client_info.version, init.protocol_version
    );
    let negotiated_version = negotiate_version(&init.protocol_version);

    let result = InitializeResult {
        protocol_version: negotiated_version,
        capabilities: ServerCapabilities {
            tools: Some(ToolsCapability {
                list_changed: Some(false),
            }),
            resources: Some(ResourcesCapability {
                subscribe: Some(false),
                list_changed: Some(false),
            }),
            prompts: Some(PromptsCapability {
                list_changed: Some(false),
            }),
            logging: Some(LoggingCapability {}),
            completions: None,
            tasks: Some(json!({
                "list": {},
                "cancel": {},
                "requests": {"tools": {"call": {}}}
            })),
            experimental: None,
        },
        server_info: Some(ServerInfo {
            name: "agent-browser-mcp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: Some("Agent Browser MCP".to_string()),
            description: Some("Browser automation tools for AI agents".to_string()),
        }),
        instructions: Some("Use browser tools to automate web interactions. Start with browser_navigate, then use browser_snapshot to understand the page structure.".to_string()),
    };

    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id.clone(), value),
        Err(e) => JsonRpcResponse::error_response(
            id,
            ERR_INTERNAL,
            &format!("Failed to serialize initialize result: {}", e),
        ),
    }
}

/// 处理 tools/list 请求
fn handle_tools_list(id: Option<Value>) -> JsonRpcResponse {
    let tools = get_tool_definitions();
    let result = ToolsListResult {
        tools,
        next_cursor: None,
    };
    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(e) => JsonRpcResponse::error_response(
            id,
            ERR_INTERNAL,
            &format!("Serialization error: {}", e),
        ),
    }
}

/// 处理 tools/call 请求
async fn handle_tools_call(
    state: &Arc<ServerState>,
    id: Option<Value>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let params: ToolCallParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error_response(
                    id,
                    ERR_INVALID_PARAMS,
                    &format!("Invalid tool call params: {}", e),
                );
            }
        },
        None => {
            return JsonRpcResponse::error_response(id, ERR_INVALID_PARAMS, "Missing params");
        }
    };

    let arguments = params
        .arguments
        .unwrap_or(Value::Object(Default::default()));
    let arguments_map = match arguments.as_object() {
        Some(arguments) => arguments.clone(),
        None => {
            return JsonRpcResponse::error_response(
                id,
                ERR_INVALID_PARAMS,
                "Tool arguments must be a JSON object",
            );
        }
    };
    let progress_token = params
        .meta
        .as_ref()
        .and_then(|meta| meta.progress_token.clone());

    if !is_tool_enabled(&params.name) {
        return JsonRpcResponse::error_response(
            id,
            ERR_METHOD_NOT_FOUND,
            &format!(
                "Tool is unavailable under BROWSER_MCP_CAPS: {}",
                params.name
            ),
        );
    }

    if let Some(token) = &progress_token {
        state.send_progress(token, 0.0, 1.0, "Tool execution started");
    }

    if let Some(task_request) = params.task {
        if !is_task_supported(&params.name) {
            return JsonRpcResponse::error_response(
                id,
                ERR_INVALID_PARAMS,
                &format!("Tool does not support task execution: {}", params.name),
            );
        }
        let task = start_tool_task(
            state.clone(),
            params.name,
            arguments_map,
            task_request,
            progress_token,
        )
        .await;
        return JsonRpcResponse::success(id, json!({"task": task}));
    }

    if params.name == "browser_screenshot" {
        let response = handle_screenshot_tool(state, id, &arguments_map).await;
        if let Some(token) = &progress_token {
            state.send_progress(token, 1.0, 1.0, "Tool execution completed");
        }
        return response;
    }

    let response = match execute_tool(state, &params.name, arguments_map).await {
        Ok(result) => {
            let text = serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "Tool completed".to_string());
            let tool_result = ToolCallResult {
                content: vec![Content::text(text)],
                is_error: false,
                structured_content: Some(result),
            };
            match serde_json::to_value(tool_result) {
                Ok(value) => JsonRpcResponse::success(id, value),
                Err(e) => JsonRpcResponse::error_response(
                    id,
                    ERR_INTERNAL,
                    &format!("Serialization error: {}", e),
                ),
            }
        }
        Err(e) => {
            let tool_result = ToolCallResult {
                content: vec![Content::text(format!("Error: {}", e))],
                is_error: true,
                structured_content: None,
            };
            match serde_json::to_value(tool_result) {
                Ok(value) => JsonRpcResponse::success(id, value),
                Err(e) => JsonRpcResponse::error_response(
                    id,
                    ERR_INTERNAL,
                    &format!("Serialization error: {}", e),
                ),
            }
        }
    };
    if let Some(token) = &progress_token {
        state.send_progress(token, 1.0, 1.0, "Tool execution completed");
    }
    response
}

async fn handle_screenshot_tool(
    state: &ServerState,
    id: Option<Value>,
    arguments: &serde_json::Map<String, Value>,
) -> JsonRpcResponse {
    let full_page = arguments.get("full_page").and_then(Value::as_bool);
    let selector = arguments
        .get("selector")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let screenshot = state
        .browser
        .screenshot_with_options(ScreenshotOptions {
            full_page,
            selector,
        })
        .await;

    let result = match screenshot {
        Ok(screenshot) => ToolCallResult {
            content: vec![
                Content::Image {
                    data: screenshot.data,
                    mime_type: "image/png".to_string(),
                },
                Content::text(format!(
                    "Screenshot captured: {}x{} PNG",
                    screenshot.width, screenshot.height
                )),
            ],
            is_error: false,
            structured_content: Some(json!({
                "width": screenshot.width,
                "height": screenshot.height,
                "format": screenshot.format,
            })),
        },
        Err(error) => ToolCallResult {
            content: vec![Content::text(format!("Error: {error}"))],
            is_error: true,
            structured_content: None,
        },
    };

    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(error) => JsonRpcResponse::error_response(
            id,
            ERR_INTERNAL,
            &format!("Serialization error: {error}"),
        ),
    }
}

async fn start_tool_task(
    state: Arc<ServerState>,
    tool_name: String,
    arguments: serde_json::Map<String, Value>,
    request: TaskRequest,
    progress_token: Option<Value>,
) -> TaskDescriptor {
    let ttl = request.ttl.unwrap_or(600_000).clamp(1_000, 86_400_000);
    let now = now_rfc3339();
    let descriptor = TaskDescriptor {
        task_id: uuid::Uuid::new_v4().to_string(),
        status: "working".to_string(),
        status_message: Some(format!("Running {tool_name}")),
        created_at: now.clone(),
        last_updated_at: now,
        ttl,
        poll_interval: Some(500),
    };
    let entry = Arc::new(TaskEntry {
        runtime: Mutex::new(TaskRuntime {
            descriptor: descriptor.clone(),
            result: None,
            abort_handle: None,
            expires_at: None,
        }),
        notify: Notify::new(),
    });
    state
        .tasks
        .lock()
        .await
        .insert(descriptor.task_id.clone(), entry.clone());

    let task_state = state.clone();
    let task_entry = entry.clone();
    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = start_rx.await;
        let outcome = execute_tool(&task_state, &tool_name, arguments).await;
        let (status, status_message, result) = match outcome {
            Ok(value) => (
                "completed",
                Some(format!("Completed {tool_name}")),
                tool_call_result_value(Ok(value)),
            ),
            Err(error) => {
                let message = error.to_string();
                (
                    "failed",
                    Some(message.clone()),
                    tool_call_result_value(Err(message)),
                )
            }
        };

        let mut runtime = task_entry.runtime.lock().await;
        if runtime.descriptor.status == "working" {
            runtime.descriptor.status = status.to_string();
            runtime.descriptor.status_message = status_message;
            runtime.descriptor.last_updated_at = now_rfc3339();
            runtime.result = Some(result);
            runtime.abort_handle = None;
            runtime.expires_at = Some(
                tokio::time::Instant::now()
                    + std::time::Duration::from_millis(runtime.descriptor.ttl),
            );
        }
        drop(runtime);
        task_entry.notify.notify_waiters();
        if let Some(token) = progress_token {
            task_state.send_progress(
                &token,
                1.0,
                1.0,
                if status == "completed" {
                    "Task completed"
                } else {
                    "Task failed"
                },
            );
        }
    });
    entry.runtime.lock().await.abort_handle = Some(task.abort_handle());
    let _ = start_tx.send(());
    descriptor
}

async fn handle_task_get(
    state: &ServerState,
    id: Option<Value>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let params = match parse_task_id_params(params) {
        Ok(params) => params,
        Err(message) => {
            return JsonRpcResponse::error_response(id, ERR_INVALID_PARAMS, message);
        }
    };
    let Some(entry) = state.get_task(&params.task_id).await else {
        return task_not_found(id, &params.task_id);
    };
    let descriptor = entry.runtime.lock().await.descriptor.clone();
    JsonRpcResponse::success(id, json!({"task": descriptor}))
}

async fn handle_tasks_list(state: &ServerState, id: Option<Value>) -> JsonRpcResponse {
    state.prune_tasks().await;
    let entries = state
        .tasks
        .lock()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut tasks = Vec::with_capacity(entries.len());
    for entry in entries {
        tasks.push(entry.runtime.lock().await.descriptor.clone());
    }
    JsonRpcResponse::success(id, json!({"tasks": tasks}))
}

async fn handle_task_result(
    state: &ServerState,
    id: Option<Value>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let params = match parse_task_id_params(params) {
        Ok(params) => params,
        Err(message) => {
            return JsonRpcResponse::error_response(id, ERR_INVALID_PARAMS, message);
        }
    };
    let Some(entry) = state.get_task(&params.task_id).await else {
        return task_not_found(id, &params.task_id);
    };

    loop {
        let notified = entry.notify.notified();
        let runtime = entry.runtime.lock().await;
        match runtime.descriptor.status.as_str() {
            "completed" | "failed" => {
                return JsonRpcResponse::success(
                    id,
                    runtime.result.clone().unwrap_or_else(|| json!({})),
                );
            }
            "cancelled" => {
                return JsonRpcResponse::error_response(
                    id,
                    ERR_INVALID_REQUEST,
                    "Task was cancelled before producing a result",
                );
            }
            _ => {}
        }
        drop(runtime);
        notified.await;
    }
}

async fn handle_task_cancel(
    state: &ServerState,
    id: Option<Value>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let params = match parse_task_id_params(params) {
        Ok(params) => params,
        Err(message) => {
            return JsonRpcResponse::error_response(id, ERR_INVALID_PARAMS, message);
        }
    };
    let Some(entry) = state.get_task(&params.task_id).await else {
        return task_not_found(id, &params.task_id);
    };

    let mut runtime = entry.runtime.lock().await;
    if runtime.descriptor.status == "working" {
        runtime.descriptor.status = "cancelled".to_string();
        runtime.descriptor.status_message = Some("Cancelled by client".to_string());
        runtime.descriptor.last_updated_at = now_rfc3339();
        if let Some(handle) = runtime.abort_handle.take() {
            handle.abort();
        }
        runtime.expires_at = Some(
            tokio::time::Instant::now() + std::time::Duration::from_millis(runtime.descriptor.ttl),
        );
    }
    let descriptor = runtime.descriptor.clone();
    drop(runtime);
    entry.notify.notify_waiters();
    JsonRpcResponse::success(id, json!({"task": descriptor}))
}

fn parse_task_id_params(params: Option<Value>) -> std::result::Result<TaskIdParams, &'static str> {
    params
        .and_then(|params| serde_json::from_value(params).ok())
        .ok_or("Missing or invalid taskId")
}

fn task_not_found(id: Option<Value>, task_id: &str) -> JsonRpcResponse {
    JsonRpcResponse::error_response(
        id,
        ERR_RESOURCE_NOT_FOUND,
        &format!("Task not found: {task_id}"),
    )
}

fn tool_call_result_value(result: std::result::Result<Value, String>) -> Value {
    let result = match result {
        Ok(value) => ToolCallResult {
            content: vec![Content::text(
                serde_json::to_string_pretty(&value)
                    .unwrap_or_else(|_| "Tool completed".to_string()),
            )],
            is_error: false,
            structured_content: Some(value),
        },
        Err(error) => ToolCallResult {
            content: vec![Content::text(format!("Error: {error}"))],
            is_error: true,
            structured_content: None,
        },
    };
    serde_json::to_value(result).unwrap_or_else(|_| json!({"isError": true}))
}

fn now_rfc3339() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

/// 处理 resources/list 请求
fn handle_resources_list(id: Option<Value>) -> JsonRpcResponse {
    let resources = vec![
        Resource {
            uri: "resource://browser/screenshot".to_string(),
            name: "Current Page Screenshot".to_string(),
            description: Some("Screenshot of the current browser page".to_string()),
            mime_type: Some("image/png".to_string()),
            size: None,
        },
        Resource {
            uri: "resource://browser/snapshot".to_string(),
            name: "Page Accessibility Snapshot".to_string(),
            description: Some("Accessibility tree snapshot of the current page".to_string()),
            mime_type: Some("text/plain".to_string()),
            size: None,
        },
    ];

    let result = ResourcesListResult {
        resources,
        next_cursor: None,
    };

    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(e) => JsonRpcResponse::error_response(
            id,
            ERR_INTERNAL,
            &format!("Serialization error: {}", e),
        ),
    }
}

/// 处理 resources/read 请求
async fn handle_resources_read(
    state: &ServerState,
    id: Option<Value>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let params: ResourceReadParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error_response(
                    id,
                    ERR_INVALID_PARAMS,
                    &format!("Invalid resource read params: {}", e),
                );
            }
        },
        None => {
            return JsonRpcResponse::error_response(id, ERR_INVALID_PARAMS, "Missing params");
        }
    };

    let contents = match params.uri.as_str() {
        "resource://browser/screenshot" => match state.browser.screenshot().await {
            Ok(screenshot) => {
                vec![ResourceContents::Blob {
                    uri: params.uri.clone(),
                    mime_type: Some("image/png".to_string()),
                    blob: screenshot.data,
                }]
            }
            Err(e) => {
                return JsonRpcResponse::error_response(
                    id,
                    ERR_INTERNAL,
                    &format!("Failed to take screenshot: {}", e),
                );
            }
        },
        "resource://browser/snapshot" => match state.browser.snapshot().await {
            Ok(snapshot) => {
                vec![ResourceContents::Text {
                    uri: params.uri.clone(),
                    mime_type: Some("text/plain".to_string()),
                    text: format_snapshot(&snapshot),
                }]
            }
            Err(e) => {
                return JsonRpcResponse::error_response(
                    id,
                    ERR_INTERNAL,
                    &format!("Failed to get snapshot: {}", e),
                );
            }
        },
        uri => {
            return JsonRpcResponse::error_response(
                id,
                ERR_RESOURCE_NOT_FOUND,
                &format!("Resource not found: {}", uri),
            );
        }
    };

    let result = ResourceReadResult { contents };
    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(e) => JsonRpcResponse::error_response(
            id,
            ERR_INTERNAL,
            &format!("Serialization error: {}", e),
        ),
    }
}

/// 处理 prompts/list 请求
fn handle_prompts_list(id: Option<Value>) -> JsonRpcResponse {
    let prompts = vec![
        Prompt {
            name: "analyze_page".to_string(),
            description: Some("Analyze the current page structure and content".to_string()),
            arguments: Some(vec![PromptArgument {
                name: "focus_area".to_string(),
                description: Some(
                    "Area to focus on (e.g., 'forms', 'links', 'content')".to_string(),
                ),
                required: false,
            }]),
        },
        Prompt {
            name: "fill_form".to_string(),
            description: Some("Guide for filling out a form on the page".to_string()),
            arguments: Some(vec![PromptArgument {
                name: "form_data".to_string(),
                description: Some("JSON object with field names and values".to_string()),
                required: true,
            }]),
        },
        Prompt {
            name: "extract_data".to_string(),
            description: Some("Extract structured data from the page".to_string()),
            arguments: Some(vec![PromptArgument {
                name: "selectors".to_string(),
                description: Some("CSS selectors for data to extract".to_string()),
                required: false,
            }]),
        },
    ];

    let result = PromptsListResult {
        prompts,
        next_cursor: None,
    };

    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(e) => JsonRpcResponse::error_response(
            id,
            ERR_INTERNAL,
            &format!("Serialization error: {}", e),
        ),
    }
}

/// 处理 prompts/get 请求
fn handle_prompts_get(id: Option<Value>, params: Option<Value>) -> JsonRpcResponse {
    let params: PromptGetParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error_response(
                    id,
                    ERR_INVALID_PARAMS,
                    &format!("Invalid prompt get params: {}", e),
                );
            }
        },
        None => {
            return JsonRpcResponse::error_response(id, ERR_INVALID_PARAMS, "Missing params");
        }
    };

    let messages = match params.name.as_str() {
        "analyze_page" => {
            let focus = params
                .arguments
                .as_ref()
                .and_then(|a| a.get("focus_area"))
                .map(|v| v.as_str())
                .unwrap_or("all");

            vec![PromptMessage {
                role: "user".to_string(),
                content: Content::text(format!(
                    "Please analyze the current page, focusing on: {}. Use browser_snapshot to get the page structure.",
                    focus
                )),
            }]
        }
        "fill_form" => {
            let form_data = params
                .arguments
                .as_ref()
                .and_then(|a| a.get("form_data"))
                .map(|v| v.to_string())
                .unwrap_or_else(|| "{}".to_string());

            vec![PromptMessage {
                role: "user".to_string(),
                content: Content::text(format!(
                    "Fill out the form on the current page with the following data: {}. First use browser_snapshot to identify form fields, then use browser_type for each field.",
                    form_data
                )),
            }]
        }
        "extract_data" => {
            let selectors = params
                .arguments
                .as_ref()
                .and_then(|a| a.get("selectors"))
                .map(|v| v.to_string())
                .unwrap_or_else(|| "all text content".to_string());

            vec![PromptMessage {
                role: "user".to_string(),
                content: Content::text(format!(
                    "Extract data from the page using selectors: {}. Use browser_snapshot to understand the page structure, then browser_evaluate to extract the data.",
                    selectors
                )),
            }]
        }
        name => {
            return JsonRpcResponse::error_response(
                id,
                ERR_INVALID_PARAMS,
                &format!("Unknown prompt: {}", name),
            );
        }
    };

    let result = PromptGetResult {
        description: Some(format!("Prompt: {}", params.name)),
        messages,
    };

    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(e) => JsonRpcResponse::error_response(
            id,
            ERR_INTERNAL,
            &format!("Serialization error: {}", e),
        ),
    }
}

/// 处理 logging/setLevel 请求
fn handle_set_log_level(id: Option<Value>, params: Option<Value>) -> JsonRpcResponse {
    let params: SetLogLevelParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error_response(
                    id,
                    ERR_INVALID_PARAMS,
                    &format!("Invalid logging params: {}", e),
                );
            }
        },
        None => {
            return JsonRpcResponse::error_response(id, ERR_INVALID_PARAMS, "Missing params");
        }
    };

    info!("Log level set to: {}", params.level);
    JsonRpcResponse::success(id, json!({}))
}

// ---------------------------------------------------------------------------
// 工具执行
// ---------------------------------------------------------------------------

/// 执行工具
async fn execute_tool(
    state: &ServerState,
    tool_name: &str,
    arguments: serde_json::Map<String, Value>,
) -> anyhow::Result<Value> {
    match tool_name {
        "browser_navigate" => {
            let url = required_string(&arguments, "url")?;
            let result = state.browser.navigate(url).await?;
            Ok(serde_json::to_value(result)?)
        }

        "browser_snapshot" => {
            let defaults = agent_browser_core::SnapshotOptions::default();
            let snapshot = state
                .browser
                .snapshot_with_options(agent_browser_core::SnapshotOptions {
                    interactive_only: arguments
                        .get("interactive_only")
                        .and_then(Value::as_bool)
                        .unwrap_or(defaults.interactive_only),
                    root_ref: arguments
                        .get("root_ref")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    max_depth: arguments
                        .get("max_depth")
                        .and_then(Value::as_u64)
                        .map(|value| value as usize)
                        .or(defaults.max_depth),
                    max_nodes: arguments
                        .get("max_nodes")
                        .and_then(Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(defaults.max_nodes)
                        .clamp(1, 5_000),
                })
                .await?;
            Ok(serde_json::to_value(snapshot)?)
        }

        "browser_snapshot_search" => {
            let query = required_string(&arguments, "query")?;
            let result = state
                .browser
                .search_snapshot(
                    query,
                    arguments
                        .get("max_results")
                        .and_then(Value::as_u64)
                        .unwrap_or(20) as usize,
                )
                .await?;
            Ok(serde_json::to_value(result)?)
        }

        "browser_click" => {
            execute_snapshot_action(state, &arguments, agent_browser_core::ActionKind::Click).await
        }

        "browser_type" => {
            let text = required_string(&arguments, "text")?.to_string();
            execute_snapshot_action(
                state,
                &arguments,
                agent_browser_core::ActionKind::Type {
                    text,
                    clear_first: arguments.get("clear_first").and_then(Value::as_bool),
                },
            )
            .await
        }

        "browser_press" => {
            let key = required_string(&arguments, "key")?.to_string();
            execute_snapshot_action(
                state,
                &arguments,
                agent_browser_core::ActionKind::Press { key },
            )
            .await
        }

        "browser_scroll" => {
            let direction = arguments
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or("down")
                .to_string();
            let amount = arguments
                .get("amount")
                .and_then(Value::as_i64)
                .unwrap_or(300) as i32;
            execute_snapshot_action(
                state,
                &arguments,
                agent_browser_core::ActionKind::Scroll {
                    direction: Some(direction),
                    amount: Some(amount),
                },
            )
            .await
        }

        "browser_wait" => {
            let timeout_ms = arguments
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(1000);
            let selector = arguments.get("selector").and_then(Value::as_str);

            if let Some(sel) = selector {
                state.browser.wait_for_selector(sel, timeout_ms).await?;
                Ok(json!({"ok": true, "selector": sel, "timeout_ms": timeout_ms}))
            } else {
                state.browser.wait(timeout_ms).await?;
                Ok(json!({"ok": true, "waited_ms": timeout_ms}))
            }
        }

        "browser_evaluate" => {
            let script = required_string(&arguments, "script")?;
            let result = state.browser.evaluate(script).await?;
            Ok(json!({"value": result}))
        }

        "browser_get_cookies" => {
            let cookies = state.browser.get_cookies().await?;
            Ok(json!({"cookies": cookies}))
        }

        "browser_set_cookies" => {
            let cookies_value = arguments
                .get("cookies")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Missing 'cookies' parameter"))?;
            let cookies: Vec<SetCookieParam> = serde_json::from_value(cookies_value)?;
            state.browser.set_cookies(cookies).await?;
            Ok(json!({"ok": true}))
        }

        "browser_list_tabs" => {
            let tabs = state.browser.list_tabs().await?;
            Ok(json!({"tabs": tabs}))
        }

        "browser_activate_tab" => {
            let tab_id = required_string(&arguments, "tab_id")?;
            state.browser.activate_tab(tab_id).await?;
            Ok(json!({"ok": true, "tab_id": tab_id}))
        }

        "browser_close_tab" => {
            let tab_id = required_string(&arguments, "tab_id")?;
            state.browser.close_tab(tab_id).await?;
            Ok(json!({"ok": true, "tab_id": tab_id}))
        }

        "browser_upload" => {
            let snapshot_id = required_string(&arguments, "snapshot_id")?;
            let ref_id = required_string(&arguments, "ref_id")?;
            let file_path = required_string(&arguments, "file_path")?;
            state
                .browser
                .upload_file_with_snapshot(snapshot_id, ref_id, file_path)
                .await?;
            Ok(json!({"ok": true, "ref_id": ref_id, "file_path": file_path}))
        }

        "browser_wait_for_network_idle" => {
            let idle_ms = arguments
                .get("idle_ms")
                .and_then(Value::as_u64)
                .unwrap_or(500);
            let timeout_ms = arguments
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(30000);
            state
                .browser
                .wait_for_network_idle(idle_ms, timeout_ms)
                .await?;
            Ok(json!({"ok": true, "idle_ms": idle_ms, "timeout_ms": timeout_ms}))
        }

        "browser_shutdown" => {
            state.browser.shutdown().await?;
            Ok(json!({"ok": true}))
        }

        "browser_enter_iframe" => {
            let snapshot_id = required_string(&arguments, "snapshot_id")?;
            let ref_id = required_string(&arguments, "ref_id")?;
            let depth = state
                .browser
                .enter_iframe_with_snapshot(snapshot_id, ref_id)
                .await?;
            Ok(json!({"ok": true, "ref_id": ref_id, "depth": depth}))
        }

        "browser_exit_iframe" => {
            let depth = state.browser.exit_iframe().await?;
            Ok(json!({"ok": true, "depth": depth}))
        }

        "browser_exit_all_iframes" => {
            state.browser.exit_all_iframes().await?;
            Ok(json!({"ok": true, "depth": 0}))
        }

        "browser_download_file" => {
            let url = required_string(&arguments, "url")?;
            let save_path = arguments.get("save_path").and_then(Value::as_str);
            let timeout_ms = arguments
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(60000);

            let options = agent_browser_core::DownloadOptions {
                save_path: save_path.map(|s| s.to_string()),
                timeout_ms: Some(timeout_ms),
            };

            let result = state.browser.download_file(url, Some(options)).await?;
            Ok(serde_json::to_value(result)?)
        }

        "browser_click_and_download" => {
            let snapshot_id = required_string(&arguments, "snapshot_id")?;
            let ref_id = required_string(&arguments, "ref_id")?;
            let save_path = arguments.get("save_path").and_then(Value::as_str);
            let timeout_ms = arguments
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(60000);

            let options = agent_browser_core::DownloadOptions {
                save_path: save_path.map(|s| s.to_string()),
                timeout_ms: Some(timeout_ms),
            };

            let result = state
                .browser
                .click_and_download_with_snapshot(snapshot_id, ref_id, Some(options))
                .await?;
            Ok(serde_json::to_value(result)?)
        }

        "browser_press_key" => {
            let key = required_string(&arguments, "key")?;

            let modifiers: Vec<agent_browser_core::KeyModifier> = arguments
                .get("modifiers")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|s| match s.to_lowercase().as_str() {
                            "alt" => Some(agent_browser_core::KeyModifier::Alt),
                            "control" | "ctrl" => Some(agent_browser_core::KeyModifier::Control),
                            "meta" | "cmd" | "command" => {
                                Some(agent_browser_core::KeyModifier::Meta)
                            }
                            "shift" => Some(agent_browser_core::KeyModifier::Shift),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();

            let result = state.browser.press_with_modifiers(key, &modifiers).await?;
            Ok(json!({"ok": result.success, "message": result.message}))
        }

        "browser_shortcut" => {
            let shortcut = required_string(&arguments, "shortcut")?;
            let result = state.browser.send_shortcut(shortcut).await?;
            Ok(json!({"ok": result.success, "message": result.message}))
        }

        "browser_navigate_with_options" => {
            let url = required_string(&arguments, "url")?;
            let wait_until = arguments
                .get("wait_until")
                .and_then(Value::as_str)
                .unwrap_or("load");

            let wait_strategy = match wait_until {
                "domContentLoaded" => agent_browser_core::NavigationWaitUntil::DomContentLoaded,
                "networkIdle" => agent_browser_core::NavigationWaitUntil::NetworkIdle,
                "none" => agent_browser_core::NavigationWaitUntil::None,
                _ => agent_browser_core::NavigationWaitUntil::Load,
            };

            let result = state
                .browser
                .navigate_with_options(url, wait_strategy)
                .await?;
            Ok(json!({
                "url": result.url,
                "title": result.title,
                "final_url": result.final_url,
                "wait_until": wait_until,
            }))
        }

        "browser_enable_network_monitoring" => {
            state.browser.enable_network_monitoring().await?;
            Ok(json!({"ok": true}))
        }

        "browser_get_network_requests" => {
            let requests = state.browser.get_network_requests().await?;
            Ok(json!({"requests": requests}))
        }

        "browser_clear_network_requests" => {
            state.browser.clear_network_requests().await?;
            Ok(json!({"ok": true}))
        }

        "browser_enable_console_monitoring" => {
            state.browser.enable_console_monitoring().await?;
            Ok(json!({"ok": true}))
        }

        "browser_get_console_messages" => {
            let messages = state.browser.get_console_messages().await?;
            Ok(json!({"messages": messages}))
        }

        "browser_clear_console_messages" => {
            state.browser.clear_console_messages().await?;
            Ok(json!({"ok": true}))
        }

        "browser_set_viewport" => {
            let width = arguments
                .get("width")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("Missing 'width' parameter"))?
                as u32;
            let height = arguments
                .get("height")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("Missing 'height' parameter"))?
                as u32;
            let device_scale_factor = arguments.get("device_scale_factor").and_then(Value::as_f64);

            let viewport = agent_browser_core::ViewportSize {
                width,
                height,
                device_scale_factor,
            };

            state.browser.set_viewport(&viewport).await?;
            Ok(json!({"ok": true, "viewport": viewport}))
        }

        "browser_get_viewport" => {
            let viewport = state.browser.get_viewport_size().await?;
            Ok(serde_json::to_value(viewport)?)
        }

        "browser_new_tab" => {
            let url = required_string(&arguments, "url")?;
            let tab = state.browser.new_tab(url).await?;
            Ok(serde_json::to_value(tab)?)
        }

        "browser_find" => {
            let strategy = required_string(&arguments, "strategy")?;
            let query = required_string(&arguments, "query")?;
            let timeout = arguments.get("timeout_ms").and_then(Value::as_u64);
            let ref_id = match strategy {
                "role" => {
                    state
                        .browser
                        .find_by_role(
                            query,
                            arguments.get("name").and_then(Value::as_str),
                            timeout,
                        )
                        .await?
                }
                "text" => state.browser.find_by_text(query, timeout).await?,
                "label" => state.browser.find_by_label(query, timeout).await?,
                _ => anyhow::bail!("Unknown find strategy: {strategy}"),
            };
            Ok(json!({
                "ref_id": ref_id,
                "snapshot_id": state.browser.current_snapshot_id().await,
                "strategy": strategy,
                "query": query,
            }))
        }

        "browser_dialog" => {
            let accept = arguments
                .get("accept")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            state
                .browser
                .setup_dialog_handler(
                    accept,
                    arguments
                        .get("prompt_text")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                )
                .await?;
            Ok(json!({"ok": true, "accept": accept}))
        }

        "browser_network" => {
            let operation = required_string(&arguments, "operation")?;
            match operation {
                "block" => {
                    let pattern = required_string(&arguments, "pattern")?;
                    state.browser.intercept_requests(pattern, true).await?;
                    Ok(json!({"ok": true, "operation": operation, "pattern": pattern}))
                }
                "unblock" => {
                    let pattern = required_string(&arguments, "pattern")?;
                    state.browser.intercept_requests(pattern, false).await?;
                    Ok(json!({"ok": true, "operation": operation, "pattern": pattern}))
                }
                "clear" => {
                    state.browser.disable_interception().await?;
                    Ok(json!({"ok": true, "operation": operation}))
                }
                "list" => Ok(json!({
                    "patterns": state.browser.blocked_request_patterns().await
                })),
                "response_body" => {
                    let request_id = required_string(&arguments, "request_id")?;
                    let body = state.browser.get_response_body(request_id).await?;
                    Ok(json!({"request_id": request_id, "body": body}))
                }
                _ => anyhow::bail!("Unknown network operation: {operation}"),
            }
        }

        "browser_emulate_device" => {
            let device = required_string(&arguments, "device")?;
            state.browser.emulate_device(device).await?;
            Ok(json!({"ok": true, "device": device}))
        }

        _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
    }
}

fn required_string<'a>(
    arguments: &'a serde_json::Map<String, Value>,
    name: &str,
) -> anyhow::Result<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing '{name}' parameter"))
}

async fn execute_snapshot_action(
    state: &ServerState,
    arguments: &serde_json::Map<String, Value>,
    action: agent_browser_core::ActionKind,
) -> anyhow::Result<Value> {
    let snapshot_id = required_string(arguments, "snapshot_id")?;
    let ref_id = arguments
        .get("ref_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let result = state
        .browser
        .act_with_snapshot(snapshot_id, ref_id, action)
        .await?;
    let snapshot = state
        .browser
        .snapshot_with_options(agent_browser_core::SnapshotOptions::default())
        .await?;
    let diff = state.browser.latest_snapshot_diff().await;
    Ok(json!({
        "ok": result.success,
        "message": result.message,
        "snapshot": snapshot,
        "diff": diff,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize_params() -> Value {
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        })
    }

    #[test]
    fn initialize_echoes_request_id() {
        let id = Some(json!(7));
        let response = handle_initialize(id.clone(), Some(initialize_params()));
        assert_eq!(response.id, id);
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn initialize_rejects_invalid_params() {
        let response = handle_initialize(Some(json!(8)), Some(json!({})));
        assert_eq!(response.id, Some(json!(8)));
        assert_eq!(response.error.unwrap().code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn task_timestamp_uses_rfc3339_utc() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        let timestamp = now_rfc3339();
        assert!(timestamp.ends_with('Z'));
        assert_eq!(timestamp.len(), 24);
    }

    #[test]
    fn initialize_advertises_task_support() {
        let response = handle_initialize(Some(json!(9)), Some(initialize_params()));
        let tasks = &response.result.unwrap()["capabilities"]["tasks"];
        assert!(tasks["requests"]["tools"]["call"].is_object());
        assert!(tasks["cancel"].is_object());
    }

    #[tokio::test]
    async fn task_can_complete_and_return_tool_result() {
        let (notification_tx, _notification_rx) = mpsc::unbounded_channel();
        let state = Arc::new(ServerState::new(notification_tx));
        let task = start_tool_task(
            state.clone(),
            "browser_wait".to_string(),
            serde_json::Map::from_iter([("timeout_ms".to_string(), json!(1))]),
            TaskRequest::default(),
            None,
        )
        .await;

        let response = handle_task_result(
            &state,
            Some(json!(10)),
            Some(json!({"taskId": task.task_id})),
        )
        .await;
        assert_eq!(response.result.unwrap()["isError"], false);
    }

    #[tokio::test]
    async fn task_can_be_cancelled() {
        let (notification_tx, _notification_rx) = mpsc::unbounded_channel();
        let state = Arc::new(ServerState::new(notification_tx));
        let task = start_tool_task(
            state.clone(),
            "browser_wait".to_string(),
            serde_json::Map::from_iter([("timeout_ms".to_string(), json!(60_000))]),
            TaskRequest::default(),
            None,
        )
        .await;

        let response = handle_task_cancel(
            &state,
            Some(json!(11)),
            Some(json!({"taskId": task.task_id})),
        )
        .await;
        assert_eq!(response.result.unwrap()["task"]["status"], "cancelled");
    }
}
