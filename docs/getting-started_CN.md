# 快速开始

本指南将帮助你快速上手 Agent Browser。

## 前置要求

- **Rust** 1.85 或更高版本
- **Chrome** 或 **Chromium** 浏览器（自动检测）

## 安装

### 从源码构建

```bash
git clone https://github.com/EchoYue/agent-browser.git
cd agent-browser
cargo build --release
```

构建产物位于 `target/release/`：
- `agent-browser-mcp` - MCP 服务端（STDIO 传输）
- `agent-browser-http` - HTTP API 服务端

## 使用方式

### MCP Server 配置

将 Agent Browser 配置为 AI 助手的 MCP 服务器。

Agent Browser 实现 [MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) 规范，包含：
- 30+ 浏览器自动化工具，带行为注解
- 截图和快照作为资源访问
- 预定义提示词用于常见任务
- 可配置的日志级别

#### Claude Code

编辑 `~/.claude/settings.json`：

```json
{
  "mcpServers": {
    "browser": {
      "command": "/path/to/agent-browser-mcp"
    }
  }
}
```

#### Cursor

编辑 Cursor 设置：

```json
{
  "mcpServers": {
    "browser": {
      "command": "/path/to/agent-browser-mcp"
    }
  }
}
```

#### 其他 MCP 客户端

对于任何兼容 MCP 的客户端，添加服务器配置：
- **Command**: `agent-browser-mcp` 二进制文件路径
- **Protocol**: MCP 2025-11-25（STDIO 传输）

配置完成后，你可以让 AI 助手浏览网页：

```
请打开 example.com 并截图
```

AI 会自动调用 `browser_navigate` 和 `browser_screenshot` 工具。

### HTTP API Server

启动 HTTP 服务器以使用 RESTful API：

```bash
# 使用默认设置启动
./target/release/agent-browser-http

# 使用自定义设置启动
BROWSER_HTTP_PORT=8080 \
BROWSER_HEADLESS=1 \
BROWSER_API_KEY=your-secret-key \
BROWSER_DEFAULT_TIMEOUT_MS=60000 \
./target/release/agent-browser-http
```

#### 快速测试

```bash
# 健康检查
curl http://localhost:3000/health

# 导航到页面
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'

# 获取页面快照
curl http://localhost:3000/snapshot | jq '.data.title'

# 截图
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > screenshot.png
```

### Rust 库

在你的 Rust 项目中直接使用 Agent Browser。

#### 添加依赖

```toml
[dependencies]
agent-browser-core = "0.2"
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

#### 基本使用

```rust
use agent_browser_core::{BrowserEngine, BrowserConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 创建浏览器引擎（有头模式显示浏览器窗口）
    let engine = BrowserEngine::new(BrowserConfig::headed());

    // 导航到页面
    engine.navigate("https://example.com").await?;

    // 获取页面快照
    let snapshot = engine.snapshot().await?;
    println!("标题: {}", snapshot.title);
    println!("节点数: {}", snapshot.nodes.len());

    // 使用 ref_id 点击元素
    engine.click("ax1").await?;

    // 或使用 CSS 选择器直接操作（推荐）
    engine.click_selector("button.submit", None).await?;

    // 输入文本
    engine.type_selector("input[name='email']", "hello@example.com", true, None).await?;

    // 截图
    let screenshot = engine.screenshot().await?;

    // 关闭
    engine.shutdown().await?;

    Ok(())
}
```

#### 反检测配置

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

// 新版无头模式 + 反检测脚本
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)  // Chrome 112+ 无头模式，难以检测
    .with_stealth(true);                // 注入反检测脚本

let engine = BrowserEngine::new(config);
```

#### 自定义配置

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_browser_path("/path/to/chrome")
    .with_profile_dir("/path/to/profile")  // 持久化 Cookie
    .with_stealth(true)
    .with_arg("--disable-web-security");

let engine = BrowserEngine::new(config);
```

## 环境变量

### HTTP Server

| 变量 | 描述 | 默认值 |
|------|------|--------|
| `BROWSER_HTTP_HOST` | 监听地址 | `127.0.0.1` |
| `BROWSER_HTTP_PORT` | 服务器端口 | `3000` |
| `BROWSER_HEADLESS` | 浏览器显示模式 | 新无头 |
| `BROWSER_API_KEY` | API 认证密钥 | - |
| `BROWSER_DEFAULT_TIMEOUT_MS` | 默认超时时间（毫秒） | `30000` |
| `BROWSER_ALLOWED_FILE_ROOTS` | 上传/下载允许目录 | 当前目录和临时目录 |

## 下一步

- [API 参考](./api-reference_CN.md) - 完整的 API 文档
- [配置指南](./configuration_CN.md) - 详细的配置选项
- [使用示例](./examples_CN.md) - 更多使用示例
- [架构设计](./architecture_CN.md) - 系统设计
