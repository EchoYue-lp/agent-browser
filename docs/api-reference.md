# API Reference

Complete API documentation for Agent Browser.

## MCP Protocol Support

Agent Browser implements [MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) specification:

- **Protocol Version**: `2025-11-25`
- **Supported Versions**: `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`
- Automatic version negotiation with clients

### Server Capabilities

| Capability | Description |
|------------|-------------|
| **Tools** | 30+ browser automation tools with annotations |
| **Resources** | Screenshot and snapshot as resources |
| **Prompts** | Pre-defined prompts for common tasks |
| **Logging** | Configurable log levels |
| **Tasks** | Durable tool execution with get/list/result/cancel methods |

## MCP Tools

Agent Browser provides 30+ MCP tools for AI agents.

Ref-based tools (`browser_click`, `browser_type`, `browser_press`, `browser_scroll`, uploads, iframe entry, and click-download) require both the `snapshot_id` and `ref_id` returned by `browser_snapshot`. A new observation invalidates earlier references.

### Tool Annotations

Each tool includes behavior annotations:

- **`readOnlyHint`**: Tool only reads data, no side effects
- **`destructiveHint`**: Tool may cause irreversible changes
- **`idempotentHint`**: Same input always produces same result
- **`openWorldHint`**: Tool interacts with external systems

### Navigation & Page

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_navigate` | Navigate to URL | `openWorldHint: true` |
| `browser_navigate_with_options` | Navigate with wait strategy | `openWorldHint: true` |
| `browser_snapshot` | Get Accessibility Tree snapshot | `readOnlyHint: true` |
| `browser_screenshot` | Take screenshot | `readOnlyHint: true` |
| `browser_wait` | Wait for selector/timeout | `readOnlyHint: true` |
| `browser_wait_for_network_idle` | Wait for network idle | `readOnlyHint: true` |

### Element Actions

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_click` | Click element (by ref_id) | `openWorldHint: true` |
| `browser_type` | Type text into element | `openWorldHint: true` |
| `browser_press` | Press key on element | `openWorldHint: true` |
| `browser_press_key` | Press key with modifiers | `openWorldHint: true` |
| `browser_shortcut` | Send keyboard shortcut | `openWorldHint: true` |
| `browser_scroll` | Scroll page | `idempotentHint: true` |
| `browser_upload` | Upload file | `openWorldHint: true` |

### Tabs & Frames

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_list_tabs` | List all tabs | `readOnlyHint: true` |
| `browser_activate_tab` | Switch to tab | `idempotentHint: true` |
| `browser_close_tab` | Close tab | `destructiveHint: true` |
| `browser_enter_iframe` | Enter iframe context | - |
| `browser_exit_iframe` | Exit iframe context | `idempotentHint: true` |
| `browser_exit_all_iframes` | Exit all iframes | `idempotentHint: true` |

### Network & Console Monitoring

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_enable_network_monitoring` | Enable network monitoring | `idempotentHint: true` |
| `browser_get_network_requests` | Get captured requests | `readOnlyHint: true` |
| `browser_clear_network_requests` | Clear request records | `idempotentHint: true` |
| `browser_enable_console_monitoring` | Enable console monitoring | `idempotentHint: true` |
| `browser_get_console_messages` | Get console messages | `readOnlyHint: true` |
| `browser_clear_console_messages` | Clear console messages | `idempotentHint: true` |

### Downloads & Cookies

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_download_file` | Download file from URL | `openWorldHint: true` |
| `browser_click_and_download` | Click and wait for download | `openWorldHint: true` |
| `browser_get_cookies` | Get cookies | `readOnlyHint: true` |
| `browser_set_cookies` | Set cookies | - |

### Viewport & Advanced

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_evaluate` | Execute JavaScript | `openWorldHint: true` |
| `browser_set_viewport` | Set viewport size | `idempotentHint: true` |
| `browser_get_viewport` | Get viewport size | `readOnlyHint: true` |
| `browser_shutdown` | Close browser | `destructiveHint: true` |

## MCP Resources

Access browser state as MCP resources:

| Resource URI | Description | MIME Type |
|--------------|-------------|-----------|
| `resource://browser/screenshot` | Current page screenshot | `image/png` |
| `resource://browser/snapshot` | Accessibility tree snapshot | `text/plain` |

## MCP Tasks, Progress, and Cancellation

Task-capable tools advertise `execution.taskSupport: "optional"`. Pass `task: {"ttl": 600000}` to `tools/call`, then use `tasks/get`, `tasks/list`, `tasks/result`, or `tasks/cancel`. Requests with `_meta.progressToken` receive `notifications/progress`; `notifications/cancelled` aborts the matching in-flight JSON-RPC request.

Set `BROWSER_MCP_CAPS` to a comma-separated subset of `network`, `storage`, `files`, and `devtools`. Disabled tools are neither listed nor callable.

### Reading Resources

Resources can be accessed via `resources/read`:

```json
{
  "method": "resources/read",
  "params": {
    "uri": "resource://browser/screenshot"
  }
}
```

**Response:**

```json
{
  "contents": [
    {
      "type": "blob",
      "uri": "resource://browser/screenshot",
      "mimeType": "image/png",
      "blob": "base64-encoded-image-data"
    }
  ]
}
```

## MCP Prompts

Pre-defined prompts for common browser tasks:

| Prompt | Description | Arguments |
|--------|-------------|-----------|
| `analyze_page` | Analyze page structure and content | `focus_area` (optional) |
| `fill_form` | Guide for filling out forms | `form_data` (required) |
| `extract_data` | Extract structured data from page | `selectors` (optional) |

### Using Prompts

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

**Response:**

```json
{
  "description": "Guide for filling out a web form",
  "messages": [
    {
      "role": "user",
      "content": {
        "type": "text",
        "text": "Fill the following form data: {...}"
      }
    }
  ]
}
```

## HTTP API

### Base URL

```
http://localhost:3000
```

### Authentication

Optional API key authentication via `X-API-Key` or Bearer authorization:

```bash
curl -H "X-API-Key: YOUR_API_KEY" http://localhost:3000/snapshot
```

### Isolated Sessions

Create a session with `POST /sessions`, then send its ID in `X-Browser-Session` on browser requests. Use `GET /sessions` to list sessions and `DELETE /sessions/{id}` to shut one down. Omitting the header uses the default session.

### Endpoints

#### Navigation

##### POST /navigate

Navigate to a URL.

```bash
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "wait_until": "networkIdle"}'
```

**Response:**
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

Get the Accessibility Tree snapshot of the current page.

```bash
curl http://localhost:3000/snapshot
```

**Response:**
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

#### Element Actions

##### POST /act

Perform an action on an element.

```bash
curl -X POST http://localhost:3000/act \
  -H "Content-Type: application/json" \
  -d '{"snapshot_id": "SNAPSHOT_ID", "ref_id": "ax1", "action": "click"}'
```

**Actions:**
- `click` - Click element
- `double_click` - Double click
- `right_click` - Right click
- `type` - Type text (requires `text` parameter)
- `hover` - Hover over element
- `focus` - Focus element
- `scroll` - Scroll page (requires `direction`, `amount`)

#### CSS Selector Operations

These endpoints allow direct element operations using CSS selectors without needing `ref_id`.

##### POST /click-selector

Click element by CSS selector.

```bash
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "button.submit", "timeout_ms": 5000}'
```

##### POST /type-selector

Type text into element by CSS selector.

```bash
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"email\"]", "text": "hello@example.com", "clear_first": true}'
```

##### POST /get-text

Get text content of element.

```bash
curl -X POST http://localhost:3000/get-text \
  -H "Content-Type: application/json" \
  -d '{"selector": ".article-title"}'
```

##### POST /get-attribute

Get attribute value of element.

```bash
curl -X POST http://localhost:3000/get-attribute \
  -H "Content-Type: application/json" \
  -d '{"selector": "a.download", "attribute": "href"}'
```

##### POST /element-exists

Check if element exists.

```bash
curl -X POST http://localhost:3000/element-exists \
  -H "Content-Type: application/json" \
  -d '{"selector": ".login-form"}'
```

##### POST /hover

Hover over element.

```bash
curl -X POST http://localhost:3000/hover \
  -H "Content-Type: application/json" \
  -d '{"selector": ".dropdown-trigger"}'
```

##### POST /select-option

Select option in dropdown.

```bash
# Select by value
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector': 'select#country', 'value': 'us'}'

# Select by text
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector': 'select#country', 'value': 'United States', 'by_text': true}'
```

##### POST /submenu

Expand menu and click submenu item.

```bash
curl -X POST http://localhost:3000/submenu \
  -H "Content-Type: application/json" \
  -d '{"menu_selector": ".menu-item", "submenu_selector": ".submenu .action"}'
```

#### Screenshot

##### GET /screenshot

Take a screenshot.

```bash
# Viewport screenshot
curl http://localhost:3000/screenshot | jq -r '.data.image' | base64 -d > screenshot.png

# Full page screenshot
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > full.png

# Element screenshot
curl "http://localhost:3000/screenshot?selector=.main-content" | jq -r '.data.image' | base64 -d > element.png
```

#### JavaScript Execution

##### POST /evaluate

Execute JavaScript.

```bash
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "document.title"}'
```

#### Tabs

##### GET /tabs

List all open tabs.

```bash
curl http://localhost:3000/tabs
```

##### POST /tabs/{tab_id}/activate

Activate a tab.

```bash
curl -X POST http://localhost:3000/tabs/tab-123/activate
```

##### DELETE /tabs/{tab_id}

Close a tab.

```bash
curl -X DELETE http://localhost:3000/tabs/tab-123
```

#### Cookies

##### GET /cookies

Get all cookies.

```bash
curl http://localhost:3000/cookies
```

##### POST /cookies

Set cookies.

```bash
curl -X POST http://localhost:3000/cookies \
  -H "Content-Type: application/json" \
  -d '{"cookies": [{"name": "session", "value": "abc123"}]}'
```

#### Network Monitoring

##### POST /network/enable

Enable network request monitoring.

```bash
curl -X POST http://localhost:3000/network/enable
```

##### GET /network/requests

Get captured network requests.

```bash
curl http://localhost:3000/network/requests
```

##### POST /network/clear

Clear network request records.

```bash
curl -X POST http://localhost:3000/network/clear
```

#### Console Monitoring

##### POST /console/enable

Enable console message monitoring.

```bash
curl -X POST http://localhost:3000/console/enable
```

##### GET /console/messages

Get captured console messages.

```bash
curl http://localhost:3000/console/messages
```

##### POST /console/clear

Clear console message records.

```bash
curl -X POST http://localhost:3000/console/clear
```

#### Viewport

##### POST /viewport

Set viewport size.

```bash
curl -X POST http://localhost:3000/viewport \
  -H "Content-Type: application/json" \
  -d '{"width": 1920, "height": 1080}'
```

##### GET /viewport

Get current viewport size.

```bash
curl http://localhost:3000/viewport
```

#### Advanced

##### POST /upload

Upload file to file input.

```bash
curl -X POST http://localhost:3000/upload \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax5", "file_path": "/path/to/file.pdf"}'
```

##### POST /download

Download a file.

```bash
curl -X POST http://localhost:3000/download \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.pdf", "save_path": "/downloads"}'
```

##### POST /click-download

Click element and wait for download.

```bash
curl -X POST http://localhost:3000/click-download \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax10", "save_path": "/downloads"}'
```

##### POST /press-key

Press key with optional modifiers.

```bash
curl -X POST http://localhost:3000/press-key \
  -H "Content-Type: application/json" \
  -d '{"key": "Enter"}'

# With modifiers
curl -X POST http://localhost:3000/press-key \
  -H "Content-Type: application/json" \
  -d '{"key": "c", "modifiers": ["control"]}'
```

##### POST /shortcut

Send keyboard shortcut.

```bash
curl -X POST http://localhost:3000/shortcut \
  -H "Content-Type: application/json" \
  -d '{"shortcut": "Ctrl+Shift+I"}'
```

#### Utility

##### GET /health

Health check.

```bash
curl http://localhost:3000/health
```

**Response:**
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

Shutdown browser.

```bash
curl -X POST http://localhost:3000/shutdown
```

#### WebSocket

##### GET /ws

WebSocket endpoint for real-time events.

```javascript
const ws = new WebSocket('ws://localhost:3000/ws');
ws.onmessage = (event) => {
  console.log('Event:', JSON.parse(event.data));
};
```

## Rust API

### BrowserEngine

The main entry point for browser automation.

```rust
use agent_browser_core::{ActionKind, BrowserEngine, BrowserConfig, HeadlessMode};

// Create engine
let engine = BrowserEngine::new(BrowserConfig::default());

// Launch browser
engine.launch().await?;

// Navigate
engine.navigate("https://example.com").await?;

// Get snapshot
let snapshot = engine.snapshot().await?;

// Click only with the snapshot that produced the ref_id
engine
    .act_with_snapshot(&snapshot.snapshot_id, "ax1", ActionKind::Click)
    .await?;

// Click by CSS selector
engine.click_selector("button.primary", None).await?;

// Type text
engine.type_selector("input#email", "hello@example.com", true, None).await?;

// Get text
let text = engine.get_text(".article-title", None).await?;

// Screenshot
let screenshot = engine.screenshot().await?;

// Execute JavaScript
let result = engine.evaluate("document.title").await?;

// Shutdown
engine.shutdown().await?;
```

### Configuration

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

// Or use builder pattern
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_browser_path("/path/to/chrome")
    .with_profile_dir("/path/to/profile")
    .with_stealth(true)
    .with_arg("--disable-web-security");
```
