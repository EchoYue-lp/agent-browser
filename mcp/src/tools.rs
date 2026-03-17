//! MCP 工具定义

use serde_json::{Value, json};

/// 工具定义
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: fn() -> Value,
}

/// 获取所有工具定义
pub fn get_tool_definitions() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": (tool.input_schema)()
            })
        })
        .collect()
}

/// 工具列表
static TOOLS: &[ToolDefinition] = &[
    // ── 导航 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_navigate",
        description: "打开浏览器并导航到指定 URL。返回页面信息。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "目标 URL（需包含 http:// 或 https://）"
                    }
                },
                "required": ["url"]
            })
        },
    },
    // ── 快照 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_snapshot",
        description: "获取页面的 Accessibility Tree 快照。返回所有可交互元素的 ref_id、role、name。先调用此方法了解页面结构，再使用其他操作。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {}
            })
        },
    },
    // ── 点击 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_click",
        description: "点击页面元素。需要先调用 browser_snapshot 获取 ref_id。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "元素引用 ID（如 'ax1', 'e5'）"
                    }
                },
                "required": ["ref_id"]
            })
        },
    },
    // ── 输入 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_type",
        description: "在输入框中输入文本。需要先调用 browser_snapshot 获取 ref_id。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "元素引用 ID"
                    },
                    "text": {
                        "type": "string",
                        "description": "要输入的文本"
                    },
                    "clear_first": {
                        "type": "boolean",
                        "description": "是否先清空输入框，默认 false"
                    }
                },
                "required": ["ref_id", "text"]
            })
        },
    },
    // ── 按键 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_press",
        description: "在元素上按键（如 Enter、Tab、Escape）。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "元素引用 ID"
                    },
                    "key": {
                        "type": "string",
                        "description": "按键名称（如 'Enter', 'Tab', 'Escape', 'ArrowDown'）"
                    }
                },
                "required": ["ref_id", "key"]
            })
        },
    },
    // ── 滚动 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_scroll",
        description: "滚动页面。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down", "left", "right"],
                        "description": "滚动方向，默认 down"
                    },
                    "amount": {
                        "type": "integer",
                        "description": "滚动像素数，默认 300"
                    }
                }
            })
        },
    },
    // ── 截图 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_screenshot",
        description: "截取当前页面的截图。支持全页面和指定元素截图。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "full_page": {
                        "type": "boolean",
                        "description": "是否截取整个页面，默认 false"
                    },
                    "selector": {
                        "type": "string",
                        "description": "CSS 选择器，仅截取该元素"
                    }
                }
            })
        },
    },
    // ── 等待 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_wait",
        description: "等待指定时间或等待选择器出现。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "timeout_ms": {
                        "type": "integer",
                        "description": "等待时间（毫秒），默认 1000"
                    },
                    "selector": {
                        "type": "string",
                        "description": "等待该 CSS 选择器出现"
                    }
                }
            })
        },
    },
    // ── 执行 JS ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_evaluate",
        description: "在页面中执行 JavaScript 代码。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "JavaScript 代码"
                    }
                },
                "required": ["script"]
            })
        },
    },
    // ── Cookie 管理 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_get_cookies",
        description: "获取当前页面的所有 Cookie。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {}
            })
        },
    },
    ToolDefinition {
        name: "browser_set_cookies",
        description: "为当前页面设置 Cookie。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "cookies": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "value": {"type": "string"},
                                "domain": {"type": "string"},
                                "path": {"type": "string"}
                            },
                            "required": ["name", "value"]
                        }
                    }
                },
                "required": ["cookies"]
            })
        },
    },
    // ── 标签页管理 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_list_tabs",
        description: "列出所有打开的浏览器标签页。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {}
            })
        },
    },
    ToolDefinition {
        name: "browser_activate_tab",
        description: "切换到指定标签页。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "tab_id": {
                        "type": "string",
                        "description": "要激活的标签页 ID"
                    }
                },
                "required": ["tab_id"]
            })
        },
    },
    ToolDefinition {
        name: "browser_close_tab",
        description: "关闭指定标签页。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "tab_id": {
                        "type": "string",
                        "description": "要关闭的标签页 ID"
                    }
                },
                "required": ["tab_id"]
            })
        },
    },
    // ── 文件上传 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_upload",
        description: "上传文件到文件输入框。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "文件输入框的元素引用 ID"
                    },
                    "file_path": {
                        "type": "string",
                        "description": "本地文件的绝对路径"
                    }
                },
                "required": ["ref_id", "file_path"]
            })
        },
    },
    // ── 网络空闲 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_wait_for_network_idle",
        description: "等待网络请求完成（SPA 页面加载完成后使用）。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "idle_ms": {
                        "type": "integer",
                        "description": "空闲判定时间（毫秒），默认 500"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "最大等待时间（毫秒），默认 30000"
                    }
                }
            })
        },
    },
    // ── 关闭 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_shutdown",
        description: "关闭浏览器。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {}
            })
        },
    },
    // ── iframe 上下文 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_enter_iframe",
        description: "进入指定的 iframe 上下文。后续操作将在该 iframe 内执行。支持嵌套 iframe。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "iframe 元素的引用 ID（如 'iframe1'）"
                    }
                },
                "required": ["ref_id"]
            })
        },
    },
    ToolDefinition {
        name: "browser_exit_iframe",
        description: "退出当前 iframe 上下文，返回到父级上下文。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {}
            })
        },
    },
    ToolDefinition {
        name: "browser_exit_all_iframes",
        description: "退出所有 iframe 上下文，返回到主文档。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {}
            })
        },
    },
    // ── 文件下载 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_download_file",
        description: "下载指定 URL 的文件。返回下载结果，包括文件路径。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "要下载的文件 URL"
                    },
                    "save_path": {
                        "type": "string",
                        "description": "保存目录（可选，默认临时目录）"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "下载超时时间（毫秒，默认 60000）"
                    }
                },
                "required": ["url"]
            })
        },
    },
    ToolDefinition {
        name: "browser_click_and_download",
        description: "点击元素并等待下载完成。用于点击下载按钮场景。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "要点击的元素引用 ID"
                    },
                    "save_path": {
                        "type": "string",
                        "description": "保存目录（可选，默认临时目录）"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "下载超时时间（毫秒，默认 60000）"
                    }
                },
                "required": ["ref_id"]
            })
        },
    },
    // ── 键盘快捷键 ────────────────────────────────────────────────
    ToolDefinition {
        name: "browser_press_key",
        description: "按键（支持修饰键组合）。可执行 Ctrl+C、Cmd+S 等快捷键。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "按键（如 'c', 'Enter', 'F5'）"
                    },
                    "modifiers": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["alt", "control", "meta", "shift"] },
                        "description": "修饰键列表（如 ['control'] 表示 Ctrl）"
                    }
                },
                "required": ["key"]
            })
        },
    },
    ToolDefinition {
        name: "browser_shortcut",
        description: "发送预设快捷键。支持：copy, paste, cut, save, selectAll, undo, redo, find, refresh, devTools, print, newTab, closeTab。",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "shortcut": {
                        "type": "string",
                        "description": "快捷键名称（如 'copy', 'save', 'undo'）"
                    }
                },
                "required": ["shortcut"]
            })
        },
    },
];
