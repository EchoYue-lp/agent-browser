# 架构设计

Agent Browser 的系统架构和设计决策。

## 概述

Agent Browser 采用模块化设计，职责分离清晰：

```
┌─────────────────────────────────────────────────────────────────┐
│                    AI Agent (MCP 客户端)                        │
│  Claude Code | Cursor | OpenAI | 自定义 Agent                   │
└────────────────────────────┬────────────────────────────────────┘
                             │ MCP 2025-11-25 (stdio)
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-mcp (MCP 服务端)                 │
│  Tools (30+) | Resources | Prompts | Logging                    │
│  协议: 2025-11-25 | 传输: stdio                                  │
└────────────────────────────┬────────────────────────────────────┘
                             │ 复用
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-core (核心库)                    │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │  Browser    │  │  Snapshot   │  │  Actions    │             │
│  │  Engine     │  │  Generator  │  │  Dispatcher │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │  Types      │  │  Error      │  │  Config     │             │
│  │             │  │  Handling   │  │             │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
└────────────────────────────┬────────────────────────────────────┘
                             │ CDP (Chrome DevTools Protocol)
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                      chromiumoxide                               │
│  Rust CDP 客户端                                                │
└────────────────────────────┬────────────────────────────────────┘
                             │ WebSocket/IPC
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Chrome / Chromium                             │
└─────────────────────────────────────────────────────────────────┘
```

## 组件

### agent-browser-core

提供浏览器自动化能力的核心库。

#### BrowserEngine

浏览器控制的主要入口点：

```rust
pub struct BrowserEngine {
    config: BrowserConfig,
    browser: Arc<Mutex<Option<Arc<Browser>>>>,
    active_page: Arc<Mutex<Option<Page>>>,
    iframe_context: Arc<Mutex<Vec<IframeContext>>>,
}
```

主要职责：
- 浏览器生命周期管理（启动、关闭）
- 页面导航和等待
- 活动页面跟踪
- Iframe 上下文管理

#### 快照生成器

从页面提取 Accessibility Tree：

```rust
pub async fn get_full_snapshot(page: &Page) -> Result<PageSnapshot>
```

为什么使用 Accessibility Tree？
- 语义化的元素角色（button、link、textbox）
- 页面重载后标识符稳定
- 内置元素可见性过滤
- 浏览器计算的无障碍名称

#### 操作分发器

执行元素操作：

```rust
pub async fn dispatch_action(
    page: &Page,
    ref_id: &str,
    action: ActionKind,
) -> Result<ActionResult>
```

支持的操作：
- Click、Double-click、Right-click
- Type、Press
- Hover、Focus
- Scroll、Drag
- Select、Wait

### agent-browser-mcp

为 AI Agent 实现的 MCP 服务端。

#### 协议处理器

基于 MCP 2025-11-25 的 JSON-RPC 2.0 实现：

```rust
fn handle_request(request: Request, state: &ServerState) -> Response
```

方法：
- `initialize` - 服务端能力与版本协商
- `tools/list` - 可用工具（带注解）
- `tools/call` - 执行工具
- `resources/list` - 可用资源
- `resources/read` - 读取资源内容
- `prompts/list` - 可用提示词
- `prompts/get` - 获取提示词消息
- `logging/setLevel` - 设置日志级别

#### 传输层

模块化传输架构：

| 传输 | 状态 | 描述 |
|------|------|------|
| **STDIO** | 生产可用 | 标准输入/输出（默认） |
| **SSE** | 客户端实现 | Server-Sent Events |
| **HTTP** | 客户端实现 | Streamable HTTP |

#### 工具定义

每个工具符合 MCP 2025-11-25 规范结构：

```rust
struct Tool {
    name: String,
    title: Option<String>,
    description: Option<String>,
    input_schema: serde_json::Value,
    output_schema: Option<Value>,
    annotations: Option<ToolAnnotations>,
}
```

#### 工具注解

工具包含行为注解：

```rust
struct ToolAnnotations {
    read_only_hint: Option<bool>,
    destructive_hint: Option<bool>,
    idempotent_hint: Option<bool>,
    open_world_hint: Option<bool>,
}
```

### agent-browser-http

提供 RESTful 访问的 HTTP API 服务器。

#### 路由

基于 Axum 的路由：

```rust
Router::new()
    .route("/navigate", post(navigate))
    .route("/snapshot", get(snapshot))
    .route("/act", post(act))
    // ...
```

#### WebSocket 支持

实时事件广播：

```rust
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response
```

## MCP 2025-11-25 特性

### 协议版本协商

```rust
pub fn negotiate_version(client_version: &str) -> String {
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&client_version) {
        client_version.to_string()
    } else {
        MCP_PROTOCOL_VERSION.to_string()  // 回退到最新版本
    }
}
```

支持版本: `2025-11-25`、`2025-06-18`、`2025-03-26`、`2024-11-05`

### 服务端能力

```rust
pub struct ServerCapabilities {
    pub tools: Option<ToolsCapability>,      // listChanged 支持
    pub resources: Option<ResourcesCapability>, // subscribe, listChanged
    pub prompts: Option<PromptsCapability>,   // listChanged
    pub logging: Option<LoggingCapability>,
}
```

### 资源

| 资源 URI | 描述 | MIME 类型 |
|----------|------|-----------|
| `resource://browser/screenshot` | 当前页面截图 | `image/png` |
| `resource://browser/snapshot` | Accessibility Tree 快照 | `text/plain` |

### 提示词

| 提示词 | 描述 | 参数 |
|--------|------|------|
| `analyze_page` | 分析页面结构 | `focus_area`（可选） |
| `fill_form` | 表单填写指南 | `form_data`（必填） |
| `extract_data` | 数据提取指南 | `selectors`（可选） |

## 设计决策

### 为什么使用 Accessibility Tree？

传统网页自动化使用：
- CSS 选择器：脆弱，DOM 变化时失效
- XPath：复杂，难以维护
- 坐标：布局变化时失效

Accessibility Tree 提供：
- **语义角色**：按用途识别元素，而非位置
- **稳定标识符**：浏览器计算，重载后一致
- **可见性过滤**：自动排除隐藏元素
- **AI 友好**：与自然语言对齐

### 为什么使用 CDP 而非 WebDriver？

| 特性 | CDP | WebDriver |
|------|-----|-----------|
| 性能 | 直接 WebSocket | HTTP 开销 |
| 能力 | 完整浏览器控制 | 受限于规范 |
| 事件 | 实时 | 需要轮询 |
| 无头模式 | 原生支持 | 需要扩展 |
| 反检测 | 可实现 | 较难 |

### 多协议支持

Agent Browser 提供三种接口：

1. **MCP (stdio)**：最适合 AI Agent
   - 零网络开销
   - 标准协议，支持 Tools、Resources、Prompts
   - 内置工具发现和注解

2. **HTTP API**：最适合集成
   - 任意 HTTP 客户端
   - 易于调试
   - WebSocket 事件

3. **Rust 库**：最适合性能
   - 零开销
   - 完整类型安全
   - 直接 API 访问

### Ref ID 系统

元素通过 `ref_id` 标识：

```javascript
// 由 Agent Browser 注入
document.querySelector('[data-agent-ref="ax42"]')
```

生成过程：
1. CDP Accessibility API 提供节点 ID
2. ID 映射到 DOM 元素
3. 注入 `data-agent-ref` 属性
4. 页面交互时保持稳定

### Iframe 处理

嵌套浏览上下文：

```rust
pub struct IframeContext {
    pub frame_id: String,
    pub parent_frame: Option<String>,
    pub name: Option<String>,
    pub src: Option<String>,
}
```

操作：
- `enter_iframe(ref_id)` - 进入 iframe 上下文
- `exit_iframe()` - 返回父级
- `exit_all_iframes()` - 重置到主框架

## 性能考虑

### 内存使用

- 每个引擎单浏览器实例
- 延迟创建页面
- 高效快照缓存

### 超时处理

所有操作支持超时：

```rust
engine.navigate("https://example.com").await?;
// 对比
engine.navigate_with_timeout("https://example.com", 5000).await?;
```

### 连接复用

- 复用 CDP 连接
- 后台事件处理
- 非阻塞操作

## 安全

### 沙箱

Agent Browser 以以下配置运行 Chrome：
- 禁用网络安全（可选）
- 无沙箱模式（用于容器）
- 自定义配置目录

### API 认证

HTTP API 支持 API 密钥：

```bash
curl -H "Authorization: Bearer secret" http://localhost:3000/snapshot
```

### 隐身模式

反检测措施：

```rust
// 注入的 JavaScript
Object.defineProperty(navigator, 'webdriver', {get: () => undefined});
```

## 扩展 Agent Browser

### 添加新工具

1. 在 `tools.rs` 中定义工具：

```rust
fn tool_my_action() -> ToolDefinition {
    ToolDefinition {
        name: "browser_my_action",
        title: Some("我的操作"),
        description: "描述",
        input_schema: || json!({
            "type": "object",
            "properties": {
                "param": {"type": "string"}
            },
            "required": ["param"]
        }),
        annotations: ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        },
    }
}
```

2. 实现处理器：

```rust
fn handle_my_action(args: Value, state: &ServerState) -> Result<Value> {
    // 实现
}
```

3. 在工具列表中注册。

### 添加新操作

1. 添加到 `ActionKind` 枚举：

```rust
pub enum ActionKind {
    // ...
    MyAction { param: String },
}
```

2. 在 `dispatch_action` 中处理：

```rust
match action {
    ActionKind::MyAction { param } => {
        // 实现
    }
}
```

## 未来计划

- **PDF 生成**：将页面转换为 PDF
- **网络拦截**：模拟/修改请求
- **性能指标**：页面加载计时
- **多浏览器**：Firefox、Safari 支持
- **视觉测试**：截图对比
- **录制**：录制和回放会话
