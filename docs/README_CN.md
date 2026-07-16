# Agent Browser

[![CI](https://github.com/EchoYue/agent-browser/actions/workflows/ci.yml/badge.svg)](https://github.com/EchoYue/agent-browser/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/agent-browser.svg)](https://crates.io/crates/agent-browser)
[![Docs.rs](https://docs.rs/agent-browser/badge.svg)](https://docs.rs/agent-browser)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

**专为 AI Agent 设计的浏览器自动化工具集。**

[English Documentation](../README.md)

## ✨ 特性

- 🤖 **AI 优先设计** - 基于 Accessibility Tree 的语义化元素定位，专为 AI Agent 优化
- 🔌 **MCP 2025-11-25** - 完整支持 MCP 2025-11-25 规范，提供 Tools、Resources、Prompts 能力
- 🚀 **高性能** - 基于 Rust + CDP 协议构建，内存占用低，响应速度快
- 🛡️ **反检测** - 支持 `--headless=new` 和 Stealth 模式绑过检测
- 🎯 **CSS 选择器操作** - 无需 ref_id，直接使用 CSS 选择器操作元素
- 📦 **零运行时依赖** - 仅需 Chrome/Chromium 浏览器

## 📦 安装

### 从源码构建

```bash
git clone https://github.com/EchoYue/agent-browser.git
cd agent-browser
cargo build --release
```

编译产物位于 `target/release/`：
- `agent-browser-mcp` - MCP 服务端（STDIO 传输）
- `agent-browser-http` - HTTP API 服务端

### 前置要求

- Rust 1.85+
- Chrome 或 Chromium 浏览器（自动检测）

## 🚀 快速开始

### 方式一：MCP 服务端（推荐）

适用于 Claude Code、Cursor 等 MCP 客户端。

**Claude Code 配置** (`~/.claude/settings.json`):

```json
{
  "mcpServers": {
    "browser": {
      "command": "/path/to/agent-browser-mcp"
    }
  }
}
```

配置完成后，直接对 AI 说：

```
请打开 example.com 并截图
```

AI 会自动调用 `browser_navigate` 和 `browser_screenshot` 工具。

### 方式二：HTTP API

适用于任何 HTTP 客户端（Python、JavaScript、curl 等）

```bash
# 启动服务
./target/release/agent-browser-http

# 导航到页面
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'

# 获取页面快照
curl http://localhost:3000/snapshot

# 截图
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > screenshot.png
```

### 方式三：Rust 库

直接在你的 Rust 项目中使用：

```rust
use agent_browser_core::{BrowserEngine, BrowserConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let engine = BrowserEngine::new(BrowserConfig::headed());
    engine.navigate("https://example.com").await?;

    let snapshot = engine.snapshot().await?;
    println!("标题: {}", snapshot.title);

    // 使用 ref_id 点击
    engine.click("ax1").await?;

    // 或直接使用 CSS 选择器（推荐）
    engine.click_selector("button.submit", None).await?;

    engine.shutdown().await?;
    Ok(())
}
```

## 🔌 MCP 协议支持

Agent Browser 实现了 [MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) 规范：

### 协议版本

- **当前版本**：`2025-11-25`
- **支持版本**：`2025-11-25`、`2025-06-18`、`2025-03-26`、`2024-11-05`
- 支持与客户端自动版本协商

### 服务端能力

| 能力 | 描述 |
|------|------|
| **Tools** | 30+ 浏览器自动化工具，带行为注解 |
| **Resources** | 截图和快照作为资源访问 |
| **Prompts** | 预定义提示词用于常见任务 |
| **Logging** | 可配置的日志级别 |

### 传输层

| 传输 | 状态 | 描述 |
|------|------|------|
| **STDIO** | ✅ 生产可用 | 标准输入/输出（默认） |
| **SSE** | 🚧 计划中 | Server-Sent Events |
| **HTTP** | 🚧 计划中 | Streamable HTTP |

## 🛠️ MCP 工具

### 导航与页面

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_navigate` | 导航到 URL | `openWorldHint: true` |
| `browser_navigate_with_options` | 带等待策略导航 | `openWorldHint: true` |
| `browser_snapshot` | 获取 Accessibility Tree 快照 | `readOnlyHint: true` |
| `browser_screenshot` | 截图 | `readOnlyHint: true` |
| `browser_wait` | 等待选择器/超时 | `readOnlyHint: true` |
| `browser_wait_for_network_idle` | 等待网络空闲 | `readOnlyHint: true` |

### 元素操作

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_click` | 点击元素（by ref_id） | - |
| `browser_type` | 输入文本 | - |
| `browser_press` | 按键 | - |
| `browser_press_key` | 带修饰键按键 | - |
| `browser_shortcut` | 发送快捷键 | - |
| `browser_scroll` | 滚动页面 | `idempotentHint: true` |
| `browser_upload` | 文件上传 | - |

### 标签页与框架

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_list_tabs` | 列出所有标签页 | `readOnlyHint: true` |
| `browser_activate_tab` | 切换标签页 | - |
| `browser_close_tab` | 关闭标签页 | `destructiveHint: true` |
| `browser_enter_iframe` | 进入 iframe | - |
| `browser_exit_iframe` | 退出 iframe | - |
| `browser_exit_all_iframes` | 退出所有 iframe | - |

### 网络与控制台

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_enable_network_monitoring` | 启用网络监控 | - |
| `browser_get_network_requests` | 获取网络请求 | `readOnlyHint: true` |
| `browser_clear_network_requests` | 清除请求记录 | - |
| `browser_enable_console_monitoring` | 启用控制台监控 | - |
| `browser_get_console_messages` | 获取控制台消息 | `readOnlyHint: true` |
| `browser_clear_console_messages` | 清除控制台消息 | - |

### 下载与 Cookie

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_download_file` | 下载文件 | - |
| `browser_click_and_download` | 点击并下载 | - |
| `browser_get_cookies` | 获取 Cookie | `readOnlyHint: true` |
| `browser_set_cookies` | 设置 Cookie | - |

### 高级功能

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_evaluate` | 执行 JavaScript | - |
| `browser_set_viewport` | 设置视口大小 | - |
| `browser_get_viewport` | 获取视口大小 | `readOnlyHint: true` |
| `browser_shutdown` | 关闭浏览器 | `destructiveHint: true` |

### 工具注解

工具包含描述其行为的注解：

- **`readOnlyHint`**：工具只读数据，无副作用
- **`destructiveHint`**：工具可能导致不可逆变更
- **`idempotentHint`**：相同输入始终产生相同结果
- **`openWorldHint`**：工具与外部系统交互

## 📚 MCP 资源

以 MCP 资源方式访问浏览器状态：

| 资源 URI | 描述 | MIME 类型 |
|----------|------|-----------|
| `resource://browser/screenshot` | 当前页面截图 | `image/png` |
| `resource://browser/snapshot` | Accessibility Tree 快照 | `text/plain` |

## 💬 MCP 提示词

预定义的浏览器任务提示词：

| 提示词 | 描述 | 参数 |
|--------|------|------|
| `analyze_page` | 分析页面结构和内容 | `focus_area`（可选） |
| `fill_form` | 填写表单指南 | `form_data`（必填） |
| `extract_data` | 从页面提取结构化数据 | `selectors`（可选） |

## 🌐 HTTP API 端点

### 基础操作

| 端点 | 方法 | 描述 |
|------|------|------|
| `/navigate` | POST | 导航到 URL |
| `/snapshot` | GET | 获取 Accessibility Tree |
| `/act` | POST | 执行元素操作 |
| `/screenshot` | GET | 截图 |
| `/wait` | POST | 等待选择器/超时 |
| `/evaluate` | POST | 执行 JavaScript |
| `/shutdown` | POST | 关闭浏览器 |
| `/health` | GET | 健康检查 |

### CSS 选择器操作（推荐）

| 端点 | 方法 | 描述 |
|------|------|------|
| `/click-selector` | POST | 通过 CSS 选择器点击 |
| `/type-selector` | POST | 通过 CSS 选择器输入 |
| `/get-text` | POST | 获取元素文本 |
| `/get-attribute` | POST | 获取元素属性 |
| `/element-exists` | POST | 检查元素是否存在 |
| `/hover` | POST | 鼠标悬停 |
| `/select-option` | POST | 选择下拉选项 |
| `/submenu` | POST | 展开菜单并点击子菜单 |

## 📖 文档

- [快速开始](./getting-started_CN.md)
- [API 参考](./api-reference_CN.md)
- [配置指南](./configuration_CN.md)
- [使用示例](./examples_CN.md)
- [架构设计](./architecture_CN.md)

**English Documentation**: [../README.md](../README.md)

## ⚙️ 配置

### 环境变量（HTTP 服务端）

```bash
BROWSER_HTTP_HOST=127.0.0.1   # 监听地址（默认仅本机回环）
BROWSER_HTTP_PORT=8080         # 服务端口（默认：3000）
BROWSER_HEADLESS=1             # 启用无头模式
BROWSER_API_KEY=secret123      # API 密钥认证
BROWSER_DEFAULT_TIMEOUT_MS=60000  # 默认超时时间（毫秒）
BROWSER_ALLOWED_FILE_ROOTS=/tmp:/path/to/workspace  # 上传/下载允许目录
```

监听非回环地址时必须配置 `BROWSER_API_KEY`。

### Rust 配置

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

// 有头模式（可见浏览器）
let config = BrowserConfig::headed();

// 无头模式（新版，更难检测）
let config = BrowserConfig::headless();

// 自定义配置
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_browser_path("/path/to/chrome")
    .with_profile_dir("/path/to/profile")  // 持久化 Cookie
    .with_stealth(true)                     // 反检测
    .with_arg("--disable-web-security");    // 额外参数
```

## 🏗️ 架构

```
┌─────────────────────────────────────────────────────────────────┐
│                    AI Agent（MCP 客户端）                        │
│  Claude Code | Cursor | OpenAI | 自定义 Agent                    │
└────────────────────────────┬────────────────────────────────────┘
                             │ MCP 2025-11-25 (stdio)
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-mcp（MCP 服务端）                 │
│  Tools (30+) | Resources | Prompts | Logging                    │
│  协议: 2025-11-25 | 传输: stdio                                  │
└────────────────────────────┬────────────────────────────────────┘
                             │ 复用
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-core（核心库）                    │
│  BrowserEngine | Accessibility Tree | Actions | Types           │
└────────────────────────────┬────────────────────────────────────┘
                             │ CDP（Chrome DevTools Protocol）
                             ▼
                      Chrome / Chromium
```

## 🔧 开发

```bash
# 开发构建
cargo build

# 发布构建
cargo build --release

# 运行测试
cargo test

# 测试 MCP 服务端
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | ./target/release/agent-browser-mcp

# 测试 HTTP 服务端
./target/release/agent-browser-http &
curl http://localhost:3000/health
```

## 📄 许可证

MIT 许可证 - 详见 [LICENSE](../LICENSE)。

## 🤝 贡献

欢迎贡献！详见 [CONTRIBUTING.md](../CONTRIBUTING.md)。

## 📝 更新日志

详见 [CHANGELOG.md](../CHANGELOG.md)。
