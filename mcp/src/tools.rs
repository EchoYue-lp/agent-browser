//! MCP 工具定义（2025-11-25 规范）
//!
//! 包含工具注解（annotations）以描述工具行为

use serde_json::{Value, json};

use crate::protocol::{Tool, ToolAnnotations};

/// 获取所有工具定义
pub fn get_tool_definitions() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|def| Tool {
            name: def.name.to_string(),
            title: def.title.map(|s| s.to_string()),
            description: Some(def.description.to_string()),
            input_schema: (def.input_schema)(),
            output_schema: None,
            annotations: Some(def.annotations.clone()),
        })
        .collect()
}

/// 工具定义
pub struct ToolDefinition {
    pub name: &'static str,
    pub title: Option<&'static str>,
    pub description: &'static str,
    pub input_schema: fn() -> Value,
    pub annotations: ToolAnnotations,
}

/// 工具列表
static TOOLS: &[ToolDefinition] = &[
    // ── 导航 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_navigate",
        title: Some("Navigate to URL"),
        description: "Open browser and navigate to the specified URL. Returns page info.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Target URL (must include http:// or https://)"
                    }
                },
                "required": ["url"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 快照 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_snapshot",
        title: Some("Get Page Snapshot"),
        description: "Get Accessibility Tree snapshot of the page. Returns ref_id, role, name for all interactive elements. Call this first to understand page structure.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 点击 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_click",
        title: Some("Click Element"),
        description: "Click an element on the page. Requires ref_id from browser_snapshot.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "Element reference ID (e.g., 'ax1', 'e5')"
                    }
                },
                "required": ["ref_id"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 输入 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_type",
        title: Some("Type Text"),
        description: "Type text into an input field. Requires ref_id from browser_snapshot.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": { "type": "string", "description": "Element reference ID" },
                    "text": { "type": "string", "description": "Text to type" },
                    "clear_first": { "type": "boolean", "description": "Clear field first (default: false)" }
                },
                "required": ["ref_id", "text"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 按键 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_press",
        title: Some("Press Key"),
        description: "Press a key on an element (e.g., Enter, Tab, Escape).",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": { "type": "string", "description": "Element reference ID" },
                    "key": { "type": "string", "description": "Key name (e.g., 'Enter', 'Tab', 'Escape', 'ArrowDown')" }
                },
                "required": ["ref_id", "key"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 滚动 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_scroll",
        title: Some("Scroll Page"),
        description: "Scroll the page in a direction.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "enum": ["up", "down", "left", "right"], "description": "Direction (default: down)" },
                    "amount": { "type": "integer", "description": "Pixels to scroll (default: 300)" }
                }
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 截图 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_screenshot",
        title: Some("Take Screenshot"),
        description: "Take a screenshot of the current page. Supports full page and element screenshots.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "full_page": { "type": "boolean", "description": "Capture full page (default: false)" },
                    "selector": { "type": "string", "description": "CSS selector to capture specific element" }
                }
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 等待 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_wait",
        title: Some("Wait"),
        description: "Wait for a specified time or for a selector to appear.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "timeout_ms": { "type": "integer", "description": "Timeout in milliseconds (default: 1000)" },
                    "selector": { "type": "string", "description": "CSS selector to wait for" }
                }
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 执行 JS ──────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_evaluate",
        title: Some("Execute JavaScript"),
        description: "Execute JavaScript code on the page.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "script": { "type": "string", "description": "JavaScript code" }
                },
                "required": ["script"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── Cookie 管理 ──────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_get_cookies",
        title: Some("Get Cookies"),
        description: "Get all cookies for the current page.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_set_cookies",
        title: Some("Set Cookies"),
        description: "Set cookies for the current page.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "cookies": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "value": { "type": "string" },
                                "domain": { "type": "string" },
                                "path": { "type": "string" }
                            },
                            "required": ["name", "value"]
                        }
                    }
                },
                "required": ["cookies"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(false),
        },
    },
    // ── 标签页管理 ──────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_list_tabs",
        title: Some("List Tabs"),
        description: "List all open browser tabs.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_activate_tab",
        title: Some("Activate Tab"),
        description: "Switch to a specific tab.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "tab_id": { "type": "string", "description": "Tab ID to activate" }
                },
                "required": ["tab_id"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_close_tab",
        title: Some("Close Tab"),
        description: "Close a specific tab.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "tab_id": { "type": "string", "description": "Tab ID to close" }
                },
                "required": ["tab_id"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(true),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 文件上传 ────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_upload",
        title: Some("Upload File"),
        description: "Upload a file to a file input element.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": { "type": "string", "description": "File input element reference ID" },
                    "file_path": { "type": "string", "description": "Absolute path to local file" }
                },
                "required": ["ref_id", "file_path"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 网络空闲 ────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_wait_for_network_idle",
        title: Some("Wait for Network Idle"),
        description: "Wait for network requests to complete (use after SPA page load).",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "idle_ms": { "type": "integer", "description": "Idle threshold in ms (default: 500)" },
                    "timeout_ms": { "type": "integer", "description": "Max wait time in ms (default: 30000)" }
                }
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 关闭 ────────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_shutdown",
        title: Some("Shutdown Browser"),
        description: "Close the browser.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(true),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── iframe 上下文 ───────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_enter_iframe",
        title: Some("Enter Iframe"),
        description: "Enter an iframe context. Subsequent operations will execute inside the iframe.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": { "type": "string", "description": "Iframe element reference ID" }
                },
                "required": ["ref_id"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_exit_iframe",
        title: Some("Exit Iframe"),
        description: "Exit current iframe context, return to parent context.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_exit_all_iframes",
        title: Some("Exit All Iframes"),
        description: "Exit all iframe contexts, return to main document.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 文件下载 ────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_download_file",
        title: Some("Download File"),
        description: "Download a file from URL.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL of file to download" },
                    "save_path": { "type": "string", "description": "Save directory (optional)" },
                    "timeout_ms": { "type": "integer", "description": "Download timeout (default: 60000)" }
                },
                "required": ["url"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    ToolDefinition {
        name: "browser_click_and_download",
        title: Some("Click and Download"),
        description: "Click an element and wait for download to complete.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": { "type": "string", "description": "Element to click" },
                    "save_path": { "type": "string", "description": "Save directory (optional)" },
                    "timeout_ms": { "type": "integer", "description": "Download timeout (default: 60000)" }
                },
                "required": ["ref_id"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 键盘快捷键 ──────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_press_key",
        title: Some("Press Key with Modifiers"),
        description: "Press a key with optional modifiers (Ctrl, Alt, Shift, Cmd).",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Key (e.g., 'c', 'Enter', 'F5')" },
                    "modifiers": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["alt", "control", "meta", "shift"] },
                        "description": "Modifier keys"
                    }
                },
                "required": ["key"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    ToolDefinition {
        name: "browser_shortcut",
        title: Some("Send Shortcut"),
        description: "Send a predefined keyboard shortcut (copy, paste, save, undo, etc.).",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "shortcut": { "type": "string", "description": "Shortcut name (copy, paste, cut, save, selectAll, undo, redo, find, refresh, devTools, print, newTab, closeTab)" }
                },
                "required": ["shortcut"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 高级导航 ────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_navigate_with_options",
        title: Some("Navigate with Options"),
        description: "Navigate with custom wait strategy (load, domContentLoaded, networkIdle, none).",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Target URL" },
                    "wait_until": { "type": "string", "enum": ["load", "domContentLoaded", "networkIdle", "none"], "description": "Wait strategy" }
                },
                "required": ["url"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    },
    // ── 网络监控 ────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_enable_network_monitoring",
        title: Some("Enable Network Monitoring"),
        description: "Enable network request monitoring.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_get_network_requests",
        title: Some("Get Network Requests"),
        description: "Get captured network requests (requires monitoring enabled).",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_clear_network_requests",
        title: Some("Clear Network Requests"),
        description: "Clear captured network request records.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 控制台监控 ──────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_enable_console_monitoring",
        title: Some("Enable Console Monitoring"),
        description: "Enable console message monitoring.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_get_console_messages",
        title: Some("Get Console Messages"),
        description: "Get captured console messages (requires monitoring enabled).",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_clear_console_messages",
        title: Some("Clear Console Messages"),
        description: "Clear captured console message records.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    // ── 视口设置 ────────────────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_set_viewport",
        title: Some("Set Viewport"),
        description: "Set browser viewport size for device simulation.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "description": "Viewport width in pixels" },
                    "height": { "type": "integer", "description": "Viewport height in pixels" },
                    "device_scale_factor": { "type": "number", "description": "Device pixel ratio (optional)" }
                },
                "required": ["width", "height"]
            })
        },
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
    ToolDefinition {
        name: "browser_get_viewport",
        title: Some("Get Viewport"),
        description: "Get current browser viewport size.",
        input_schema: || json!({ "type": "object", "properties": {} }),
        annotations: ToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        },
    },
];
