# API 参考

Agent Browser 完整的 API 文档。

## MCP 协议支持

Agent Browser 实现 [MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) 规范：

- **协议版本**: `2025-11-25`
- **支持版本**: `2025-11-25`、`2025-06-18`、`2025-03-26`、`2024-11-05`
- 自动与客户端进行版本协商

### 服务端能力

| 能力 | 描述 |
|------|------|
| **Tools** | 30+ 浏览器自动化工具，带行为注解 |
| **Resources** | 截图和快照作为资源访问 |
| **Prompts** | 预定义提示词用于常见任务 |
| **Logging** | 可配置的日志级别 |
| **Tasks** | 支持 get/list/result/cancel 的持久工具任务 |

## MCP 工具

Agent Browser 为 AI Agent 提供 30+ MCP 工具。

基于引用的工具（点击、输入、按键、滚动、上传、进入 iframe、点击下载）必须同时传入 `browser_snapshot` 返回的 `snapshot_id` 和 `ref_id`。生成新快照后，旧引用会失效。

### 工具注解

每个工具包含行为注解：

- **`readOnlyHint`**: 工具只读数据，无副作用
- **`destructiveHint`**: 工具可能导致不可逆变更
- **`idempotentHint`**: 相同输入始终产生相同结果
- **`openWorldHint`**: 工具与外部系统交互

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
| `browser_click` | 点击元素（by ref_id） | `openWorldHint: true` |
| `browser_type` | 输入文本 | `openWorldHint: true` |
| `browser_press` | 按键 | `openWorldHint: true` |
| `browser_press_key` | 带修饰键按键 | `openWorldHint: true` |
| `browser_shortcut` | 发送快捷键 | `openWorldHint: true` |
| `browser_scroll` | 滚动页面 | `idempotentHint: true` |
| `browser_upload` | 文件上传 | `openWorldHint: true` |

### 标签页与框架

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_list_tabs` | 列出所有标签页 | `readOnlyHint: true` |
| `browser_activate_tab` | 切换标签页 | `idempotentHint: true` |
| `browser_close_tab` | 关闭标签页 | `destructiveHint: true` |
| `browser_enter_iframe` | 进入 iframe | - |
| `browser_exit_iframe` | 退出 iframe | `idempotentHint: true` |
| `browser_exit_all_iframes` | 退出所有 iframe | `idempotentHint: true` |

### 网络与控制台监控

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_enable_network_monitoring` | 启用网络监控 | `idempotentHint: true` |
| `browser_get_network_requests` | 获取网络请求 | `readOnlyHint: true` |
| `browser_clear_network_requests` | 清除请求记录 | `idempotentHint: true` |
| `browser_enable_console_monitoring` | 启用控制台监控 | `idempotentHint: true` |
| `browser_get_console_messages` | 获取控制台消息 | `readOnlyHint: true` |
| `browser_clear_console_messages` | 清除控制台消息 | `idempotentHint: true` |

### 下载与 Cookie

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_download_file` | 从 URL 下载文件 | `openWorldHint: true` |
| `browser_click_and_download` | 点击并下载 | `openWorldHint: true` |
| `browser_get_cookies` | 获取 Cookie | `readOnlyHint: true` |
| `browser_set_cookies` | 设置 Cookie | - |

### 视口与高级功能

| 工具 | 描述 | 注解 |
|------|------|------|
| `browser_evaluate` | 执行 JavaScript | `openWorldHint: true` |
| `browser_set_viewport` | 设置视口大小 | `idempotentHint: true` |
| `browser_get_viewport` | 获取视口大小 | `readOnlyHint: true` |
| `browser_shutdown` | 关闭浏览器 | `destructiveHint: true` |

## MCP 资源

以 MCP 资源方式访问浏览器状态：

| 资源 URI | 描述 | MIME 类型 |
|----------|------|-----------|
| `resource://browser/screenshot` | 当前页面截图 | `image/png` |
| `resource://browser/snapshot` | Accessibility Tree 快照 | `text/plain` |

## MCP Tasks、进度与取消

支持后台执行的工具会声明 `execution.taskSupport: "optional"`。在 `tools/call` 中传入 `task: {"ttl": 600000}` 后，可使用 `tasks/get`、`tasks/list`、`tasks/result`、`tasks/cancel`。携带 `_meta.progressToken` 的请求会收到 `notifications/progress`；`notifications/cancelled` 会中止对应的进行中 JSON-RPC 请求。

可通过 `BROWSER_MCP_CAPS` 配置 `network`、`storage`、`files`、`devtools` 的逗号分隔子集。禁用的工具既不会被列出，也不能直接调用。

### 读取资源

通过 `resources/read` 访问资源：

```json
{
  "method": "resources/read",
  "params": {
    "uri": "resource://browser/screenshot"
  }
}
```

**响应：**

```json
{
  "contents": [
    {
      "type": "blob",
      "uri": "resource://browser/screenshot",
      "mimeType": "image/png",
      "blob": "base64编码的图像数据"
    }
  ]
}
```

## MCP 提示词

预定义的浏览器任务提示词：

| 提示词 | 描述 | 参数 |
|--------|------|------|
| `analyze_page` | 分析页面结构和内容 | `focus_area`（可选） |
| `fill_form` | 填写表单指南 | `form_data`（必填） |
| `extract_data` | 从页面提取结构化数据 | `selectors`（可选） |

### 使用提示词

```json
{
  "method": "prompts/get",
  "params": {
    "name": "fill_form",
    "arguments": {
      "form_data": "{\"email\": \"user@example.com\", \"password\": \"secret\"}"
    }
  }
}
```

**响应：**

```json
{
  "description": "填写网页表单的指南",
  "messages": [
    {
      "role": "user",
      "content": {
        "type": "text",
        "text": "填写以下表单数据: {...}"
      }
    }
  ]
}
```

## HTTP API

### 基础 URL

```
http://localhost:3000
```

### 认证

可选的 API 密钥认证，支持 `X-API-Key` 或 Bearer Authorization：

```bash
curl -H "X-API-Key: YOUR_API_KEY" http://localhost:3000/snapshot
```

### 隔离会话

通过 `POST /sessions` 创建会话，随后在浏览器请求中使用 `X-Browser-Session` 传入会话 ID。`GET /sessions` 列出会话，`DELETE /sessions/{id}` 关闭会话；不传 Header 时使用默认会话。

### 端点

#### 导航

##### POST /navigate

导航到 URL。

```bash
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "wait_until": "networkIdle"}'
```

**响应：**
```json
{
  "status": "ok",
  "data": {
    "url": "https://example.com",
    "title": "Example Domain"
  }
}
```

##### GET /snapshot

获取当前页面的 Accessibility Tree 快照。

```bash
curl http://localhost:3000/snapshot
```

**响应：**
```json
{
  "status": "ok",
  "data": {
    "url": "https://example.com",
    "title": "Example Domain",
    "nodes": [
      {
        "ref_id": "ax1",
        "role": "link",
        "name": "More information...",
        "focusable": true
      }
    ],
    "iframe_count": 0
  }
}
```

#### 元素操作

##### POST /act

对元素执行操作。

```bash
curl -X POST http://localhost:3000/act \
  -H "Content-Type: application/json" \
  -d '{"snapshot_id": "SNAPSHOT_ID", "ref_id": "ax1", "action": "click"}'
```

**操作类型：**
- `click` - 点击元素
- `double_click` - 双击
- `right_click` - 右键点击
- `type` - 输入文本（需要 `text` 参数）
- `hover` - 鼠标悬停
- `focus` - 聚焦元素
- `scroll` - 滚动页面（需要 `direction`, `amount`）

#### CSS 选择器操作

这些端点允许直接使用 CSS 选择器操作元素，无需 `ref_id`。

##### POST /click-selector

通过 CSS 选择器点击元素。

```bash
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "button.submit", "timeout_ms": 5000}'
```

##### POST /type-selector

通过 CSS 选择器输入文本。

```bash
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"email\"]", "text": "hello@example.com", "clear_first": true}'
```

##### POST /get-text

获取元素的文本内容。

```bash
curl -X POST http://localhost:3000/get-text \
  -H "Content-Type: application/json" \
  -d '{"selector": ".article-title"}'
```

##### POST /get-attribute

获取元素的属性值。

```bash
curl -X POST http://localhost:3000/get-attribute \
  -H "Content-Type: application/json" \
  -d '{"selector": "a.download", "attribute": "href"}'
```

##### POST /element-exists

检查元素是否存在。

```bash
curl -X POST http://localhost:3000/element-exists \
  -H "Content-Type: application/json" \
  -d '{"selector": ".login-form"}'
```

##### POST /hover

鼠标悬停在元素上。

```bash
curl -X POST http://localhost:3000/hover \
  -H "Content-Type: application/json" \
  -d '{"selector": ".dropdown-trigger"}'
```

##### POST /select-option

选择下拉选项。

```bash
# 按值选择
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector": "select#country", "value": "us"}'

# 按文本选择
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector": "select#country", "value": "United States", "by_text": true}'
```

##### POST /submenu

展开菜单并点击子菜单项。

```bash
curl -X POST http://localhost:3000/submenu \
  -H "Content-Type: application/json" \
  -d '{"menu_selector": ".menu-item", "submenu_selector": ".submenu .action"}'
```

#### 截图

##### GET /screenshot

截图。

```bash
# 视口截图
curl http://localhost:3000/screenshot | jq -r '.data.image' | base64 -d > screenshot.png

# 全页面截图
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > full.png

# 元素截图
curl "http://localhost:3000/screenshot?selector=.main-content" | jq -r '.data.image' | base64 -d > element.png
```

#### JavaScript 执行

##### POST /evaluate

执行 JavaScript。

```bash
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "document.title"}'
```

#### 标签页

##### GET /tabs

列出所有打开的标签页。

```bash
curl http://localhost:3000/tabs
```

##### POST /tabs/{tab_id}/activate

激活标签页。

```bash
curl -X POST http://localhost:3000/tabs/tab-123/activate
```

##### DELETE /tabs/{tab_id}

关闭标签页。

```bash
curl -X DELETE http://localhost:3000/tabs/tab-123
```

#### Cookie

##### GET /cookies

获取所有 Cookie。

```bash
curl http://localhost:3000/cookies
```

##### POST /cookies

设置 Cookie。

```bash
curl -X POST http://localhost:3000/cookies \
  -H "Content-Type: application/json" \
  -d '{"cookies": [{"name": "session", "value": "abc123"}]}'
```

#### 网络监控

##### POST /network/enable

启用网络请求监控。

```bash
curl -X POST http://localhost:3000/network/enable
```

##### GET /network/requests

获取捕获的网络请求。

```bash
curl http://localhost:3000/network/requests
```

##### POST /network/clear

清除网络请求记录。

```bash
curl -X POST http://localhost:3000/network/clear
```

#### 控制台监控

##### POST /console/enable

启用控制台消息监控。

```bash
curl -X POST http://localhost:3000/console/enable
```

##### GET /console/messages

获取捕获的控制台消息。

```bash
curl http://localhost:3000/console/messages
```

##### POST /console/clear

清除控制台消息记录。

```bash
curl -X POST http://localhost:3000/console/clear
```

#### 视口

##### POST /viewport

设置视口大小。

```bash
curl -X POST http://localhost:3000/viewport \
  -H "Content-Type: application/json" \
  -d '{"width": 1920, "height": 1080}'
```

##### GET /viewport

获取当前视口大小。

```bash
curl http://localhost:3000/viewport
```

#### 高级功能

##### POST /upload

上传文件到文件输入框。

```bash
curl -X POST http://localhost:3000/upload \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax5", "file_path": "/path/to/file.pdf"}'
```

##### POST /download

下载文件。

```bash
curl -X POST http://localhost:3000/download \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.pdf", "save_path": "/downloads"}'
```

##### POST /click-download

点击元素并等待下载。

```bash
curl -X POST http://localhost:3000/click-download \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax10", "save_path": "/downloads"}'
```

##### POST /press-key

按键（可带修饰键）。

```bash
curl -X POST http://localhost:3000/press-key \
  -H "Content-Type: application/json" \
  -d '{"key": "Enter"}'

# 带修饰键
curl -X POST http://localhost:3000/press-key \
  -H "Content-Type: application/json" \
  -d '{"key": "c", "modifiers": ["control"]}'
```

##### POST /shortcut

发送快捷键。

```bash
curl -X POST http://localhost:3000/shortcut \
  -H "Content-Type: application/json" \
  -d '{"shortcut": "Ctrl+Shift+I"}'
```

#### 工具端点

##### GET /health

健康检查。

```bash
curl http://localhost:3000/health
```

**响应：**
```json
{
  "status": "ok",
  "data": {
    "status": "ok",
    "version": "0.2.0"
  }
}
```

##### POST /shutdown

关闭浏览器。

```bash
curl -X POST http://localhost:3000/shutdown
```

#### WebSocket

##### GET /ws

实时事件的 WebSocket 端点。

```javascript
const ws = new WebSocket('ws://localhost:3000/ws');
ws.onmessage = (event) => {
  console.log('Event:', JSON.parse(event.data));
};
```

## Rust API

### BrowserEngine

浏览器自动化的主要入口点。

```rust
use agent_browser_core::{ActionKind, BrowserEngine, BrowserConfig, HeadlessMode};

// 创建引擎
let engine = BrowserEngine::new(BrowserConfig::default());

// 启动浏览器
engine.launch().await?;

// 导航
engine.navigate("https://example.com").await?;

// 获取快照
let snapshot = engine.snapshot().await?;

// 仅使用生成 ref_id 的快照执行点击
engine
    .act_with_snapshot(&snapshot.snapshot_id, "ax1", ActionKind::Click)
    .await?;

// 通过 CSS 选择器点击
engine.click_selector("button.primary", None).await?;

// 输入文本
engine.type_selector("input#email", "hello@example.com", true, None).await?;

// 获取文本
let text = engine.get_text(".article-title", None).await?;

// 截图
let screenshot = engine.screenshot().await?;

// 执行 JavaScript
let result = engine.evaluate("document.title").await?;

// 关闭
engine.shutdown().await?;
```

### 配置

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};
use std::path::PathBuf;

let config = BrowserConfig {
    headless: HeadlessMode::New,
    browser_path: Some(PathBuf::from("/path/to/chrome")),
    profile_dir: Some(PathBuf::from("/path/to/profile")),
    navigation_timeout_ms: 30000,
    action_timeout_ms: 10000,
    stealth: true,
    extra_args: vec!["--disable-web-security".to_string()],
};

// 或使用构建器模式
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_browser_path("/path/to/chrome")
    .with_profile_dir("/path/to/profile")
    .with_stealth(true)
    .with_arg("--disable-web-security");
```
