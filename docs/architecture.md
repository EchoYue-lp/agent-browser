# Architecture

System architecture and design decisions of Agent Browser.

## Overview

Agent Browser is designed as a modular system with clear separation of concerns:

```
┌─────────────────────────────────────────────────────────────────┐
│                    AI Agent (MCP Client)                        │
│  Claude Code | Cursor | OpenAI | Custom Agents                  │
└────────────────────────────┬────────────────────────────────────┘
                             │ MCP 2025-11-25 (stdio)
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-mcp (MCP Server)                 │
│  Tools (30+) | Resources | Prompts | Logging                    │
│  Protocol: 2025-11-25 | Transports: stdio, sse, http            │
└────────────────────────────┬────────────────────────────────────┘
                             │ Reuses
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-core (Core Library)              │
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
│  Rust CDP Client                                                │
└────────────────────────────┬────────────────────────────────────┘
                             │ WebSocket/IPC
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Chrome / Chromium                             │
└─────────────────────────────────────────────────────────────────┘
```

## Components

### agent-browser-core

The core library providing browser automation capabilities.

#### BrowserEngine

The main entry point for browser control:

```rust
pub struct BrowserEngine {
    config: BrowserConfig,
    browser: Arc<Mutex<Option<Arc<Browser>>>>,
    active_page: Arc<Mutex<Option<Page>>>,
    iframe_context: Arc<Mutex<Vec<IframeContext>>>,
}
```

Key responsibilities:
- Browser lifecycle management (launch, shutdown)
- Page navigation and waiting
- Active page tracking
- Iframe context management

#### Snapshot Generator

Extracts Accessibility Tree from pages:

```rust
pub async fn get_full_snapshot(page: &Page) -> Result<PageSnapshot>
```

Why Accessibility Tree?
- Semantic element roles (button, link, textbox)
- Stable identifiers across page reloads
- Built-in element visibility filtering
- Browser-computed accessible names

#### Actions Dispatcher

Executes element operations:

```rust
pub async fn dispatch_action(
    page: &Page,
    ref_id: &str,
    action: ActionKind,
) -> Result<ActionResult>
```

Supported actions:
- Click, Double-click, Right-click
- Type, Press
- Hover, Focus
- Scroll, Drag
- Select, Wait

### agent-browser-mcp

MCP Server implementation for AI agents.

#### Protocol Handler

JSON-RPC 2.0 implementation with MCP 2025-11-25:

```rust
fn handle_request(request: Request, state: &ServerState) -> Response
```

Methods:
- `initialize` - Server capabilities with version negotiation
- `tools/list` - Available tools with annotations
- `tools/call` - Execute tool
- `resources/list` - Available resources
- `resources/read` - Read resource content
- `prompts/list` - Available prompts
- `prompts/get` - Get prompt messages
- `logging/setLevel` - Set log level

#### Transport Layer

Modular transport architecture:

| Transport | Status | Description |
|-----------|--------|-------------|
| **STDIO** | Production | Standard input/output (default) |
| **SSE** | Client impl | Server-Sent Events |
| **HTTP** | Client impl | Streamable HTTP |

#### Tool Definitions

Each tool has MCP 2025-11-25 compliant structure:

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

#### Tool Annotations

Tools include behavior annotations:

```rust
struct ToolAnnotations {
    read_only_hint: Option<bool>,
    destructive_hint: Option<bool>,
    idempotent_hint: Option<bool>,
    open_world_hint: Option<bool>,
}
```

### agent-browser-http

HTTP API server for RESTful access.

#### Routes

Axum-based routing:

```rust
Router::new()
    .route("/navigate", post(navigate))
    .route("/snapshot", get(snapshot))
    .route("/act", post(act))
    // ...
```

#### WebSocket Support

Real-time event broadcasting:

```rust
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response
```

## MCP 2025-11-25 Features

### Protocol Version Negotiation

```rust
pub fn negotiate_version(client_version: &str) -> String {
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&client_version) {
        client_version.to_string()
    } else {
        MCP_PROTOCOL_VERSION.to_string()  // Fallback to latest
    }
}
```

Supported versions: `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`

### Server Capabilities

```rust
pub struct ServerCapabilities {
    pub tools: Option<ToolsCapability>,      // listChanged support
    pub resources: Option<ResourcesCapability>, // subscribe, listChanged
    pub prompts: Option<PromptsCapability>,   // listChanged
    pub logging: Option<LoggingCapability>,
}
```

### Resources

| Resource URI | Description | MIME Type |
|--------------|-------------|-----------|
| `resource://browser/screenshot` | Current page screenshot | `image/png` |
| `resource://browser/snapshot` | Accessibility tree snapshot | `text/plain` |

### Prompts

| Prompt | Description | Arguments |
|--------|-------------|-----------|
| `analyze_page` | Analyze page structure | `focus_area` (optional) |
| `fill_form` | Form filling guide | `form_data` (required) |
| `extract_data` | Data extraction guide | `selectors` (optional) |

## Design Decisions

### Why Accessibility Tree?

Traditional web automation uses:
- CSS Selectors: Fragile, break with DOM changes
- XPath: Complex, hard to maintain
- Coordinates: Break with layout changes

Accessibility Tree provides:
- **Semantic roles**: Elements identified by purpose, not position
- **Stable identifiers**: Browser-computed, consistent across reloads
- **Visibility filtering**: Hidden elements automatically excluded
- **Better for AI**: Natural language alignment

### Why CDP over WebDriver?

| Feature | CDP | WebDriver |
|---------|-----|-----------|
| Performance | Direct WebSocket | HTTP overhead |
| Capabilities | Full browser control | Limited by spec |
| Events | Real-time | Polling required |
| Headless | Native | Extension needed |
| Stealth | Possible | Harder |

### Multi-Protocol Support

Agent Browser provides three interfaces:

1. **MCP (stdio)**: Best for AI agents
   - Zero network overhead
   - Standard protocol with Tools, Resources, Prompts
   - Tool discovery and annotations built-in

2. **HTTP API**: Best for integration
   - Any HTTP client
   - Easy debugging
   - WebSocket for events

3. **Rust Library**: Best for performance
   - Zero overhead
   - Full type safety
   - Direct API access

### Ref ID System

Elements are identified by `ref_id`:

```javascript
// Injected by Agent Browser
document.querySelector('[data-agent-ref="ax42"]')
```

Generation:
1. CDP Accessibility API provides node IDs
2. IDs are mapped to DOM elements
3. `data-agent-ref` attribute is injected
4. Stable across page interactions

### Iframe Handling

Nested browsing contexts:

```rust
pub struct IframeContext {
    pub frame_id: String,
    pub parent_frame: Option<String>,
    pub name: Option<String>,
    pub src: Option<String>,
}
```

Operations:
- `enter_iframe(ref_id)` - Enter iframe context
- `exit_iframe()` - Return to parent
- `exit_all_iframes()` - Reset to main frame

## Performance Considerations

### Memory Usage

- Single browser instance per engine
- Lazy page creation
- Efficient snapshot caching

### Timeout Handling

All operations support timeouts:

```rust
engine.navigate("https://example.com").await?;
// vs
engine.navigate_with_timeout("https://example.com", 5000).await?;
```

### Connection Pooling

- Reuse CDP connection
- Background event handling
- Non-blocking operations

## Security

### Sandboxing

Agent Browser runs Chrome with:
- Disabled web security (optional)
- No sandbox mode (for containers)
- Custom profile directories

### API Authentication

HTTP API supports API key:

```bash
curl -H "Authorization: Bearer secret" http://localhost:3000/snapshot
```

### Stealth Mode

Anti-detection measures:

```rust
// Injected JavaScript
Object.defineProperty(navigator, 'webdriver', {get: () => undefined});
```

## Extending Agent Browser

### Adding New Tools

1. Define tool in `tools.rs`:

```rust
fn tool_my_action() -> ToolDefinition {
    ToolDefinition {
        name: "browser_my_action",
        title: Some("My Action"),
        description: "Description here",
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

2. Implement handler:

```rust
fn handle_my_action(args: Value, state: &ServerState) -> Result<Value> {
    // Implementation
}
```

3. Register in tools list.

### Adding New Actions

1. Add to `ActionKind` enum:

```rust
pub enum ActionKind {
    // ...
    MyAction { param: String },
}
```

2. Handle in `dispatch_action`:

```rust
match action {
    ActionKind::MyAction { param } => {
        // Implementation
    }
}
```

## Future Plans

- **PDF Generation**: Convert pages to PDF
- **Network Interception**: Mock/modify requests
- **Performance Metrics**: Page load timing
- **Multi-Browser**: Firefox, Safari support
- **Visual Testing**: Screenshot comparison
- **Recording**: Record and replay sessions