# Agent Browser

[![CI](https://github.com/EchoYue/agent-browser/actions/workflows/ci.yml/badge.svg)](https://github.com/EchoYue/agent-browser/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/agent-browser.svg)](https://crates.io/crates/agent-browser)
[![Docs.rs](https://docs.rs/agent-browser/badge.svg)](https://docs.rs/agent-browser)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

**Browser automation toolkit designed for AI Agents.**

[中文文档](./docs/README_CN.md)

## ✨ Features

- 🤖 **AI-First Design** - Semantic element location via Accessibility Tree, optimized for AI agents
- 🔌 **Multi-Protocol Support** - MCP Server + HTTP API, compatible with Claude Code, Cursor, OpenAI, etc.
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
- `agent-browser-mcp` - MCP Server
- `agent-browser-http` - HTTP API Server

### Prerequisites

- Rust 1.85+
- Chrome or Chromium browser (auto-detected)

## 🚀 Quick Start

### Option 1: MCP Server (Recommended)

For MCP clients like Claude Code, Cursor, etc.

**Claude Code Configuration** (`~/.claude/config.json`):

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
use agent_browser_core::{BrowserEngine, BrowserConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let engine = BrowserEngine::new(BrowserConfig::headed());
    engine.navigate("https://example.com").await?;

    let snapshot = engine.snapshot().await?;
    println!("Title: {}", snapshot.title);

    // Click using ref_id
    engine.click("ax1").await?;

    // Or use CSS selector directly (recommended)
    engine.click_selector("button.submit", None).await?;

    engine.shutdown().await?;
    Ok(())
}
```

## 🛠️ MCP Tools

| Tool | Description |
|------|-------------|
| `browser_navigate` | Navigate to URL |
| `browser_snapshot` | Get Accessibility Tree snapshot |
| `browser_click` | Click element (by ref_id) |
| `browser_type` | Type text |
| `browser_press` | Press key |
| `browser_scroll` | Scroll page |
| `browser_screenshot` | Take screenshot |
| `browser_wait` | Wait for selector or timeout |
| `browser_evaluate` | Execute JavaScript |
| `browser_get_cookies` | Get cookies |
| `browser_set_cookies` | Set cookies |
| `browser_list_tabs` | List tabs |
| `browser_activate_tab` | Switch tab |
| `browser_close_tab` | Close tab |
| `browser_upload` | File upload |
| `browser_shutdown` | Close browser |

## 🌐 HTTP API Endpoints

### Basic Operations

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/navigate` | POST | Navigate to URL |
| `/snapshot` | GET | Get Accessibility Tree |
| `/act` | POST | Perform element action |
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
BROWSER_HTTP_PORT=8080         # Server port (default: 3000)
BROWSER_HEADLESS=1             # Enable headless mode
BROWSER_API_KEY=secret123      # API key for authentication
BROWSER_DEFAULT_TIMEOUT_MS=60000  # Default timeout in ms
```

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
                             │ MCP Protocol (stdio)
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                   agent-browser-mcp (MCP Server)                 │
│  17 Tools: navigate, snapshot, click, type, screenshot...       │
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
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | ./target/release/agent-browser-mcp

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