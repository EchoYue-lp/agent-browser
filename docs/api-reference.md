# API Reference

Complete API documentation for Agent Browser.

## MCP Tools

Agent Browser provides 17 MCP tools for AI agents.

### Navigation

| Tool | Description | Parameters |
|------|-------------|------------|
| `browser_navigate` | Navigate to URL | `url` (string), `wait_until` (optional) |
| `browser_snapshot` | Get Accessibility Tree snapshot | - |

### Element Actions

| Tool | Description | Parameters |
|------|-------------|------------|
| `browser_click` | Click element | `ref_id` (string) |
| `browser_type` | Type text into element | `ref_id`, `text`, `clear_first` (optional) |
| `browser_press` | Press key | `ref_id`, `key` |
| `browser_scroll` | Scroll page | `direction`, `amount` |
| `browser_hover` | Hover over element | `ref_id` |

### Page Operations

| Tool | Description | Parameters |
|------|-------------|------------|
| `browser_screenshot` | Take screenshot | `full_page` (optional), `selector` (optional) |
| `browser_wait` | Wait for condition | `selector` (optional), `timeout_ms` (optional) |
| `browser_evaluate` | Execute JavaScript | `script` |

### Tab Management

| Tool | Description | Parameters |
|------|-------------|------------|
| `browser_list_tabs` | List all tabs | - |
| `browser_activate_tab` | Switch to tab | `tab_id` |
| `browser_close_tab` | Close tab | `tab_id` |

### Cookies & Storage

| Tool | Description | Parameters |
|------|-------------|------------|
| `browser_get_cookies` | Get all cookies | - |
| `browser_set_cookies` | Set cookies | `cookies` (array) |

### Advanced

| Tool | Description | Parameters |
|------|-------------|------------|
| `browser_upload` | Upload file | `ref_id`, `file_path` |
| `browser_shutdown` | Close browser | - |

## HTTP API

### Base URL

```
http://localhost:3000
```

### Authentication

Optional API key authentication via `Authorization` header:

```bash
curl -H "Authorization: Bearer YOUR_API_KEY" http://localhost:3000/snapshot
```

### Endpoints

#### Navigation

##### POST /navigate

Navigate to a URL.

```bash
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "wait_until": "networkidle0"}'
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
  -d '{"ref_id": "ax1", "action": "click"}'
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
  -d '{"selector": "select#country", "value": "us"}'

# Select by text
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector": "select#country", "value": "United States", "by_text": true}'
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
    "version": "0.1.0"
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
use agent_browser_core::{BrowserEngine, BrowserConfig, HeadlessMode};

// Create engine
let engine = BrowserEngine::new(BrowserConfig::default())?;

// Launch browser
engine.launch().await?;

// Navigate
engine.navigate("https://example.com").await?;

// Get snapshot
let snapshot = engine.snapshot().await?;

// Click by ref_id
engine.click("ax1").await?;

// Click by CSS selector
engine.click_selector("button.primary", None).await?;

// Type text
engine.type_selector("input#email", "hello@example.com", true, None).await?;

// Get text
let text = engine.get_text(".article-title", None).await?;

// Screenshot
let screenshot = engine.screenshot(None).await?;

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