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

use std::sync::Arc;

use agent_browser_core::snapshot::format_snapshot;
use agent_browser_core::{BrowserConfig, BrowserEngine, ScreenshotOptions, SetCookieParam};
use serde_json::{Value, json};
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
}

impl ServerState {
    fn new() -> Self {
        Self {
            browser: BrowserEngine::new(BrowserConfig::default()),
            initialized: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn is_initialized(&self) -> bool {
        self.initialized.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set_initialized(&self) {
        self.initialized
            .store(true, std::sync::atomic::Ordering::SeqCst);
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

    let state = Arc::new(ServerState::new());
    let transport = transport::stdio::StdioTransport::new();

    let notification_state = state.clone();
    transport
        .run(
            |request| {
                let state = state.clone();
                async move { handle_request(&state, request).await }
            },
            move |notification| handle_notification(&notification_state, &notification),
        )
        .await
}

/// 处理 MCP 请求
async fn handle_request(state: &ServerState, request: JsonRpcRequest) -> JsonRpcResponse {
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

        "tools/call" => handle_tools_call(state, id, request.params).await,

        // ── 资源 ─────────────────────────────────────────────────────────────
        "resources/list" => handle_resources_list(id),

        "resources/read" => handle_resources_read(state, id, request.params).await,

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
    state: &ServerState,
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

    if params.name == "browser_screenshot" {
        return handle_screenshot_tool(state, id, &arguments_map).await;
    }

    match execute_tool(state, &params.name, arguments_map).await {
        Ok(result) => {
            let tool_result = ToolCallResult {
                content: vec![Content::text(result)],
                is_error: false,
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
    }
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
) -> anyhow::Result<String> {
    match tool_name {
        "browser_navigate" => {
            let url = arguments["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;
            let result = state.browser.navigate(url).await?;
            Ok(format!(
                "Navigated to {}\nTitle: {}\nFinal URL: {}",
                result.url, result.title, result.final_url
            ))
        }

        "browser_snapshot" => {
            let snapshot = state.browser.snapshot().await?;
            Ok(format_snapshot(&snapshot))
        }

        "browser_click" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;
            let result = state.browser.click(ref_id).await?;
            Ok(result.message)
        }

        "browser_type" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;
            let text = arguments["text"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'text' parameter"))?;
            let clear_first = arguments["clear_first"].as_bool().unwrap_or(false);
            let result = state.browser.type_text(ref_id, text, clear_first).await?;
            Ok(result.message)
        }

        "browser_press" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;
            let key = arguments["key"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'key' parameter"))?;
            let result = state.browser.press(ref_id, key).await?;
            Ok(result.message)
        }

        "browser_scroll" => {
            let direction = arguments["direction"].as_str().unwrap_or("down");
            let amount = arguments["amount"].as_i64().unwrap_or(300) as i32;
            let result = state.browser.scroll(direction, amount).await?;
            Ok(result.message)
        }

        "browser_wait" => {
            let timeout_ms = arguments["timeout_ms"].as_u64().unwrap_or(1000);
            let selector = arguments["selector"].as_str();

            if let Some(sel) = selector {
                state.browser.wait_for_selector(sel, timeout_ms).await?;
                Ok(format!("Element '{}' appeared", sel))
            } else {
                state.browser.wait(timeout_ms).await?;
                Ok(format!("Waited {}ms", timeout_ms))
            }
        }

        "browser_evaluate" => {
            let script = arguments["script"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'script' parameter"))?;
            let result = state.browser.evaluate(script).await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }

        "browser_get_cookies" => {
            let cookies = state.browser.get_cookies().await?;
            Ok(serde_json::to_string_pretty(&cookies)?)
        }

        "browser_set_cookies" => {
            let cookies_value = arguments["cookies"].clone();
            let cookies: Vec<SetCookieParam> = serde_json::from_value(cookies_value)?;
            state.browser.set_cookies(cookies).await?;
            Ok("Cookies set successfully".to_string())
        }

        "browser_list_tabs" => {
            let tabs = state.browser.list_tabs().await?;
            Ok(serde_json::to_string_pretty(&tabs)?)
        }

        "browser_activate_tab" => {
            let tab_id = arguments["tab_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'tab_id' parameter"))?;
            state.browser.activate_tab(tab_id).await?;
            Ok(format!("Activated tab: {}", tab_id))
        }

        "browser_close_tab" => {
            let tab_id = arguments["tab_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'tab_id' parameter"))?;
            state.browser.close_tab(tab_id).await?;
            Ok(format!("Closed tab: {}", tab_id))
        }

        "browser_upload" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;
            let file_path = arguments["file_path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' parameter"))?;
            state.browser.upload_file(ref_id, file_path).await?;
            Ok(format!("File uploaded: {} -> {}", file_path, ref_id))
        }

        "browser_wait_for_network_idle" => {
            let idle_ms = arguments["idle_ms"].as_u64().unwrap_or(500);
            let timeout_ms = arguments["timeout_ms"].as_u64().unwrap_or(30000);
            state
                .browser
                .wait_for_network_idle(idle_ms, timeout_ms)
                .await?;
            Ok("Network idle detected".to_string())
        }

        "browser_shutdown" => {
            state.browser.shutdown().await?;
            Ok("Browser closed".to_string())
        }

        "browser_enter_iframe" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;
            let depth = state.browser.enter_iframe(ref_id).await?;
            Ok(format!("Entered iframe, depth: {}", depth))
        }

        "browser_exit_iframe" => {
            let depth = state.browser.exit_iframe().await?;
            Ok(format!("Exited iframe, depth: {}", depth))
        }

        "browser_exit_all_iframes" => {
            state.browser.exit_all_iframes().await?;
            Ok("Exited all iframes, returned to main document".to_string())
        }

        "browser_download_file" => {
            let url = arguments["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;
            let save_path = arguments["save_path"].as_str();
            let timeout_ms = arguments["timeout_ms"].as_u64().unwrap_or(60000);

            let options = agent_browser_core::DownloadOptions {
                save_path: save_path.map(|s| s.to_string()),
                timeout_ms: Some(timeout_ms),
            };

            let result = state.browser.download_file(url, Some(options)).await?;
            Ok(format!(
                "Download complete: {} -> {}\nSize: {} bytes\nStatus: {:?}",
                result.guid,
                result.file_path,
                result.size.unwrap_or(0),
                result.status
            ))
        }

        "browser_click_and_download" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;
            let save_path = arguments["save_path"].as_str();
            let timeout_ms = arguments["timeout_ms"].as_u64().unwrap_or(60000);

            let options = agent_browser_core::DownloadOptions {
                save_path: save_path.map(|s| s.to_string()),
                timeout_ms: Some(timeout_ms),
            };

            let result = state
                .browser
                .click_and_download(ref_id, Some(options))
                .await?;
            Ok(format!(
                "Download complete: {} -> {}\nSize: {} bytes\nStatus: {:?}",
                result.guid,
                result.file_path,
                result.size.unwrap_or(0),
                result.status
            ))
        }

        "browser_press_key" => {
            let key = arguments["key"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'key' parameter"))?;

            let modifiers: Vec<agent_browser_core::KeyModifier> = arguments["modifiers"]
                .as_array()
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
            Ok(result.message)
        }

        "browser_shortcut" => {
            let shortcut = arguments["shortcut"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'shortcut' parameter"))?;
            let result = state.browser.send_shortcut(shortcut).await?;
            Ok(result.message)
        }

        "browser_navigate_with_options" => {
            let url = arguments["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;
            let wait_until = arguments["wait_until"].as_str().unwrap_or("load");

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
            Ok(format!(
                "Navigated to {} (wait: {})\nTitle: {}\nFinal URL: {}",
                result.url, wait_until, result.title, result.final_url
            ))
        }

        "browser_enable_network_monitoring" => {
            state.browser.enable_network_monitoring().await?;
            Ok("Network monitoring enabled".to_string())
        }

        "browser_get_network_requests" => {
            let requests = state.browser.get_network_requests().await?;
            Ok(serde_json::to_string_pretty(&requests)?)
        }

        "browser_clear_network_requests" => {
            state.browser.clear_network_requests().await?;
            Ok("Network requests cleared".to_string())
        }

        "browser_enable_console_monitoring" => {
            state.browser.enable_console_monitoring().await?;
            Ok("Console monitoring enabled".to_string())
        }

        "browser_get_console_messages" => {
            let messages = state.browser.get_console_messages().await?;
            Ok(serde_json::to_string_pretty(&messages)?)
        }

        "browser_clear_console_messages" => {
            state.browser.clear_console_messages().await?;
            Ok("Console messages cleared".to_string())
        }

        "browser_set_viewport" => {
            let width = arguments["width"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'width' parameter"))?
                as u32;
            let height = arguments["height"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'height' parameter"))?
                as u32;
            let device_scale_factor = arguments["device_scale_factor"].as_f64();

            let viewport = agent_browser_core::ViewportSize {
                width,
                height,
                device_scale_factor,
            };

            state.browser.set_viewport(&viewport).await?;
            Ok(format!("Viewport set to {}x{}", width, height))
        }

        "browser_get_viewport" => {
            let viewport = state.browser.get_viewport_size().await?;
            Ok(format!(
                "Current viewport: {}x{}, device scale: {}",
                viewport.width,
                viewport.height,
                viewport.device_scale_factor.unwrap_or(1.0)
            ))
        }

        _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
    }
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
}
