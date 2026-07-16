# Agent Browser

[![CI](https://github.com/EchoYue/agent-browser/actions/workflows/ci.yml/badge.svg)](https://github.com/EchoYue/agent-browser/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/agent-browser.svg)](https://crates.io/crates/agent-browser)
[![Docs.rs](https://docs.rs/agent-browser/badge.svg)](https://docs.rs/agent-browser)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)

**Browser automation toolkit designed for AI Agents.**

[中文文档](./docs/README_CN.md)

## ✨ Features

- 🤖 **AI-First Design** - Semantic element location via Accessibility Tree, optimized for AI agents
- 🔌 **MCP 2025-11-25** - Full support for MCP 2025-11-25 specification with Tools, Resources, and Prompts
- 🚀 **High Performance** - Built with Rust + CDP protocol, low memory footprint, fast response
- 🛡️ **Anti-Detection** - Supports `--headless=new` and Stealth mode to bypass detection
- 🎯 **CSS Selector Operations** - Direct element operations using CSS selectors without ref_id
- 📦 **Zero Runtime Dependencies** - Only requires Chrome/Chromium browser

## 📦 Installation

### Build from Source

```bash
git clone https://github.com/EchoYue/agent-browser.git
cd agent-browser
cargo build --release
```

Binaries available at `target/release/`:
- `agent-browser-mcp` - MCP Server (STDIO transport)
- `agent-browser-http` - HTTP API Server

### Prerequisites

- Rust 1.88+
- Chrome or Chromium browser (auto-detected)

## 🚀 Quick Start

### Option 1: MCP Server (Recommended)

For MCP clients like Claude Code, Cursor, etc.

**Claude Code Configuration** (`~/.claude/settings.json`):

```json
{
  "mcpServers": {
    "browser": {
      "command": "/path/to/agent-browser-mcp"
    }
  }
}
```

Once configured, simply ask your AI:

```
Please open example.com and take a screenshot
```

The AI will automatically call `browser_navigate` and `browser_screenshot` tools.

### Option 2: HTTP API

For any HTTP client (Python, JavaScript, curl, etc.)

```bash
# Start server
./target/release/agent-browser-http

# Navigate to page
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'

# Get page snapshot
curl http://localhost:3000/snapshot

# Take screenshot
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > screenshot.png
```

### Option 3: Rust Library

Use directly in your Rust project:

```rust
use agent_browser_core::{ActionKind, BrowserEngine, BrowserConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let engine = BrowserEngine::new(BrowserConfig::headed());
    engine.navigate("https://example.com").await?;

    let snapshot = engine.snapshot().await?;
    println!("Title: {}", snapshot.title);

    // Bind actions to the observation that produced the ref_id.
    engine
        .act_with_snapshot(&snapshot.snapshot_id, "ax1", ActionKind::Click)
        .await?;

    // Or use CSS selector directly (recommended)
    engine.click_selector("button.submit", None).await?;

    engine.shutdown().await?;
    Ok(())
}
```

## 🔌 MCP Protocol Support

Agent Browser implements the [MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) specification:

### Protocol Version

- **Current Version**: `2025-11-25`
- **Supported Versions**: `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`
- Automatic version negotiation with clients

### Server Capabilities

| Capability | Description |
|------------|-------------|
| **Tools** | 30+ browser automation tools with annotations |
| **Resources** | Screenshot and snapshot as resources |
| **Prompts** | Pre-defined prompts for common tasks |
| **Logging** | Configurable log levels |
| **Tasks** | Durable execution, polling, result retrieval, and cancellation |

### Transport

| Transport | Status | Description |
|-----------|--------|-------------|
| **STDIO** | ✅ Production | Standard input/output (default) |
| **SSE** | 🚧 Planned | Server-Sent Events |
| **HTTP** | 🚧 Planned | Streamable HTTP |

## 🛠️ MCP Tools

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
| `browser_click` | Click by `snapshot_id` + `ref_id` | - |
| `browser_type` | Type by `snapshot_id` + `ref_id` | - |
| `browser_press` | Press key by `snapshot_id` + `ref_id` | - |
| `browser_press_key` | Press key with modifiers | - |
| `browser_shortcut` | Send keyboard shortcut | - |
| `browser_scroll` | Scroll page | `idempotentHint: true` |
| `browser_upload` | Upload file | - |

### Tabs & Frames

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_list_tabs` | List all tabs | `readOnlyHint: true` |
| `browser_activate_tab` | Switch to tab | - |
| `browser_close_tab` | Close tab | `destructiveHint: true` |
| `browser_enter_iframe` | Enter iframe context | - |
| `browser_exit_iframe` | Exit iframe context | - |
| `browser_exit_all_iframes` | Exit all iframes | - |

### Network & Console

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_enable_network_monitoring` | Enable network monitoring | - |
| `browser_get_network_requests` | Get captured requests | `readOnlyHint: true` |
| `browser_clear_network_requests` | Clear request records | - |
| `browser_enable_console_monitoring` | Enable console monitoring | - |
| `browser_get_console_messages` | Get console messages | `readOnlyHint: true` |
| `browser_clear_console_messages` | Clear console messages | - |

### Downloads & Cookies

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_download_file` | Download file from URL | - |
| `browser_click_and_download` | Click and wait for download | - |
| `browser_get_cookies` | Get cookies | `readOnlyHint: true` |
| `browser_set_cookies` | Set cookies | - |

### Advanced

| Tool | Description | Annotations |
|------|-------------|-------------|
| `browser_evaluate` | Execute JavaScript | - |
| `browser_set_viewport` | Set viewport size | - |
| `browser_get_viewport` | Get viewport size | `readOnlyHint: true` |
| `browser_shutdown` | Close browser | `destructiveHint: true` |

### Tool Annotations

Tools include annotations describing their behavior:

- **`readOnlyHint`**: Tool only reads data, no side effects
- **`destructiveHint`**: Tool may cause irreversible changes
- **`idempotentHint`**: Same input always produces same result
- **`openWorldHint`**: Tool interacts with external systems

## 📚 MCP Resources

Access browser state as MCP resources:

| Resource URI | Description | MIME Type |
|--------------|-------------|-----------|
| `resource://browser/screenshot` | Current page screenshot | `image/png` |
| `resource://browser/snapshot` | Accessibility tree snapshot | `text/plain` |

## 💬 MCP Prompts

Pre-defined prompts for common browser tasks:

| Prompt | Description | Arguments |
|--------|-------------|-----------|
| `analyze_page` | Analyze page structure and content | `focus_area` (optional) |
| `fill_form` | Guide for filling out forms | `form_data` (required) |
| `extract_data` | Extract structured data from page | `selectors` (optional) |

## 🌐 HTTP API Endpoints

### Basic Operations

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/navigate` | POST | Navigate to URL |
| `/snapshot` | GET | Get a compact Accessibility Tree and `snapshot_id` |
| `/snapshot/search` | POST | Search the latest snapshot |
| `/act` | POST | Perform a snapshot-bound element action |
| `/sessions` | POST/GET | Create or list isolated browser sessions |
| `/sessions/{id}` | DELETE | Shut down an isolated session |
| `/screenshot` | GET | Take screenshot |
| `/wait` | POST | Wait for selector/timeout |
| `/evaluate` | POST | Execute JavaScript |
| `/shutdown` | POST | Close browser |
| `/health` | GET | Health check |

### CSS Selector Operations (Recommended)

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/click-selector` | POST | Click by CSS selector |
| `/type-selector` | POST | Type by CSS selector |
| `/get-text` | POST | Get element text |
| `/get-attribute` | POST | Get element attribute |
| `/element-exists` | POST | Check element exists |
| `/hover` | POST | Mouse hover |
| `/select-option` | POST | Select dropdown option |
| `/submenu` | POST | Expand menu and click submenu |

## 📖 Documentation

- [Getting Started](./docs/getting-started.md)
- [API Reference](./docs/api-reference.md)
- [Configuration](./docs/configuration.md)
- [Examples](./docs/examples.md)
- [Architecture](./docs/architecture.md)

**中文文档**: [docs/README_CN.md](./docs/README_CN.md)

## ⚙️ Configuration

### Environment Variables (HTTP Server)

```bash
BROWSER_HTTP_HOST=127.0.0.1   # Bind address (default: loopback only)
BROWSER_HTTP_PORT=8080         # Server port (default: 3000)
BROWSER_HEADLESS=1             # Enable headless mode
BROWSER_NO_SANDBOX=false       # Explicitly disable Chrome sandbox only when required
BROWSER_API_KEY=secret123      # API key for authentication
BROWSER_DEFAULT_TIMEOUT_MS=60000  # Default timeout in ms
BROWSER_ALLOWED_FILE_ROOTS=/tmp:/path/to/workspace  # Upload/download roots
BROWSER_ALLOWED_ORIGINS=https://example.com,https://*.example.org
BROWSER_BLOCKED_ORIGINS=https://admin.example.com
BROWSER_ALLOW_PRIVATE_NETWORKS=false  # Blocks loopback/private/link-local targets by default
BROWSER_CAPTURE_SENSITIVE_DATA=false  # Redacts auth/cookie/request bodies by default
BROWSER_MCP_CAPS=network,storage,files,devtools  # Optional MCP capability allowlist
```

Binding to a non-loopback address requires `BROWSER_API_KEY`.
HTTP clients can select an isolated session with the `X-Browser-Session` header.

### Rust Configuration

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

// Headed mode (visible browser)
let config = BrowserConfig::headed();

// Headless mode (new, harder to detect)
let config = BrowserConfig::headless();

// Custom configuration
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_browser_path("/path/to/chrome")
    .with_profile_dir("/path/to/profile")  // Persist cookies
    .with_stealth(true)                     // Anti-detection
    .with_arg("--disable-web-security");    // Extra args
```

## 🏗️ Architecture

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
│  Protocol: 2025-11-25 | Transport: stdio                        │
└────────────────────────────┬────────────────────────────────────┘
                             │ Reuses
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-core (Core Library)              │
│  BrowserEngine | Accessibility Tree | Actions | Types           │
└────────────────────────────┬────────────────────────────────────┘
                             │ CDP (Chrome DevTools Protocol)
                             ▼
                      Chrome / Chromium
```

## 🔧 Development

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Test MCP server
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | ./target/release/agent-browser-mcp

# Test HTTP server
./target/release/agent-browser-http &
curl http://localhost:3000/health
```

## 📄 License

MIT License - see [LICENSE](LICENSE) for details.

## 🤝 Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## 📝 Changelog

See [CHANGELOG.md](CHANGELOG.md) for version history.
