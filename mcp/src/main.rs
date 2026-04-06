//! Echo Browser MCP Server
//!
//! 基于 MCP 协议的浏览器控制服务器。
//!
//! ## 支持的传输
//!
//! - **stdio**: 标准输入/输出（默认）
//! - **SSE**: Server-Sent Events（计划中）
//!
//! ## 使用方式
//!
//! ### Claude Code 配置
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "browser": {
//!       "command": "/path/to/echo-browser-mcp"
//!     }
//!   }
//! }
//! ```
//!
//! ### echo-agent 配置
//!
//! ```yaml
//! servers:
//!   - name: browser
//!     transport:
//!       type: stdio
//!       command: /path/to/echo-browser-mcp
//! ```
//!
//! ## 工具列表
//!
//! | 工具名 | 描述 |
//! |--------|------|
//! | browser_navigate | 导航到 URL |
//! | browser_snapshot | 获取页面快照 |
//! | browser_click | 点击元素 |
//! | browser_type | 输入文本 |
//! | browser_press | 按键 |
//! | browser_scroll | 滚动页面 |
//! | browser_screenshot | 截图 |
//! | browser_wait | 等待 |
//! | browser_evaluate | 执行 JavaScript |

use std::sync::Arc;

use agent_browser_core::snapshot::format_snapshot;
use agent_browser_core::{BrowserConfig, BrowserEngine};
use serde_json::{Value, json};
use tracing::{error, info};

mod protocol;
mod tools;

use protocol::*;
use tools::*;

// ---------------------------------------------------------------------------
// MCP Server 实现
// ---------------------------------------------------------------------------

/// MCP Server 状态
struct ServerState {
    browser: BrowserEngine,
}

impl ServerState {
    fn new() -> Self {
        Self {
            browser: BrowserEngine::new(BrowserConfig::default()),
        }
    }
}

/// 运行 MCP Server（stdio 模式）
async fn run_server() -> anyhow::Result<()> {
    info!("Starting Echo Browser MCP Server (stdio mode)");

    let state = Arc::new(ServerState::new());

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::stdout();

    // 读取 JSON-RPC 消息
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            continue;
        }

        info!("Received: {}", line);

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to parse request: {}", e);
                continue;
            }
        };

        let response = handle_request(&state, request).await;
        let response_json = serde_json::to_string(&response)?;

        info!("Sending: {}", response_json);

        stdout.write_all(response_json.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

/// 处理 MCP 请求
async fn handle_request(state: &ServerState, request: JsonRpcRequest) -> JsonRpcResponse {
    match request.method.as_str() {
        // 初始化
        "initialize" => JsonRpcResponse::success(
            request.id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "echo-browser-mcp",
                    "version": "0.1.0"
                }
            }),
        ),

        // 列出工具
        "tools/list" => JsonRpcResponse::success(
            request.id,
            json!({
                "tools": get_tool_definitions()
            }),
        ),

        // 调用工具
        "tools/call" => {
            let params = request.params.unwrap_or_default();
            let tool_name = params["name"].as_str().unwrap_or("");
            let arguments = params["arguments"].as_object().cloned().unwrap_or_default();

            match execute_tool(state, tool_name, arguments).await {
                Ok(result) => JsonRpcResponse::success(
                    request.id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": result
                        }]
                    }),
                ),
                Err(e) => JsonRpcResponse::error(request.id, -32603, &e.to_string()),
            }
        }

        // 未知方法
        _ => JsonRpcResponse::error(
            request.id,
            -32601,
            &format!("Method not found: {}", request.method),
        ),
    }
}

/// 执行工具
async fn execute_tool(
    state: &ServerState,
    tool_name: &str,
    arguments: serde_json::Map<String, Value>,
) -> anyhow::Result<String> {
    use agent_browser_core::{ScreenshotOptions, SetCookieParam};

    match tool_name {
        "browser_navigate" => {
            let url = arguments["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

            let result = state.browser.navigate(url).await?;
            Ok(format!(
                "已导航到 {}\n页面标题: {}\n最终 URL: {}",
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

        "browser_screenshot" => {
            let full_page = arguments["full_page"].as_bool();
            let selector = arguments["selector"].as_str().map(|s| s.to_string());

            let result = if full_page.is_some() || selector.is_some() {
                state
                    .browser
                    .screenshot_with_options(ScreenshotOptions {
                        full_page,
                        selector,
                    })
                    .await?
            } else {
                state.browser.screenshot().await?
            };

            Ok(format!(
                "截图成功: {}x{} PNG, {} bytes",
                result.width,
                result.height,
                result.data.len()
            ))
        }

        "browser_wait" => {
            let timeout_ms = arguments["timeout_ms"].as_u64().unwrap_or(1000);
            let selector = arguments["selector"].as_str();

            if let Some(sel) = selector {
                state.browser.wait_for_selector(sel, timeout_ms).await?;
                Ok(format!("元素 '{}' 已出现", sel))
            } else {
                state.browser.wait(timeout_ms).await?;
                Ok(format!("等待了 {}ms", timeout_ms))
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
            Ok("Cookie 设置成功".to_string())
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
            Ok(format!("已激活标签页: {}", tab_id))
        }

        "browser_close_tab" => {
            let tab_id = arguments["tab_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'tab_id' parameter"))?;

            state.browser.close_tab(tab_id).await?;
            Ok(format!("已关闭标签页: {}", tab_id))
        }

        "browser_upload" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;
            let file_path = arguments["file_path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' parameter"))?;

            state.browser.upload_file(ref_id, file_path).await?;
            Ok(format!("文件上传成功: {} -> {}", file_path, ref_id))
        }

        "browser_wait_for_network_idle" => {
            let idle_ms = arguments["idle_ms"].as_u64().unwrap_or(500);
            let timeout_ms = arguments["timeout_ms"].as_u64().unwrap_or(30000);

            state
                .browser
                .wait_for_network_idle(idle_ms, timeout_ms)
                .await?;
            Ok("网络空闲检测完成".to_string())
        }

        "browser_shutdown" => {
            state.browser.shutdown().await?;
            Ok("浏览器已关闭".to_string())
        }

        "browser_enter_iframe" => {
            let ref_id = arguments["ref_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'ref_id' parameter"))?;

            let depth = state.browser.enter_iframe(ref_id).await?;
            Ok(format!("已进入 iframe，当前深度: {}", depth))
        }

        "browser_exit_iframe" => {
            let depth = state.browser.exit_iframe().await?;
            Ok(format!("已退出 iframe，当前深度: {}", depth))
        }

        "browser_exit_all_iframes" => {
            state.browser.exit_all_iframes().await?;
            Ok("已退出所有 iframe，返回主文档".to_string())
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
                "下载完成: {} -> {}\n大小: {} 字节\n状态: {:?}",
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
                "下载完成: {} -> {}\n大小: {} 字节\n状态: {:?}",
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

        _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
    }
}

// ---------------------------------------------------------------------------
// 主入口
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr) // 日志输出到 stderr，避免干扰 stdio 协议
        .init();

    run_server().await
}
