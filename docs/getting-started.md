# Getting Started

This guide will help you get Agent Browser up and running quickly.

## Prerequisites

- **Rust** 1.85 or later
- **Chrome** or **Chromium** browser (automatically detected)

## Installation

### Build from Source

```bash
git clone https://github.com/EchoYue/agent-browser.git
cd agent-browser
cargo build --release
```

The binaries will be available at `target/release/`:
- `agent-browser-mcp` - MCP Server (STDIO transport)
- `agent-browser-http` - HTTP API Server

## Usage

### MCP Server Setup

Configure Agent Browser as an MCP server for AI assistants.

Agent Browser implements [MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) specification with:
- 30+ browser automation tools with annotations
- Resources for screenshot and snapshot access
- Pre-defined prompts for common tasks
- Logging capability

#### Claude Code

Edit `~/.claude/settings.json`:

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

Edit your Cursor settings:

```json
{
  "mcpServers": {
    "browser": {
      "command": "/path/to/agent-browser-mcp"
    }
  }
}
```

#### Other MCP Clients

For any MCP-compatible client, add the server configuration with:
- **Command**: Path to `agent-browser-mcp` binary
- **Protocol**: MCP 2025-11-25 (STDIO transport)

Once configured, you can ask your AI assistant to browse the web:

```
Please open example.com and take a screenshot
```

The AI will automatically call `browser_navigate` and `browser_screenshot` tools.

### HTTP API Server

Start the HTTP server for RESTful API access:

```bash
# Start with default settings
./target/release/agent-browser-http

# Start with custom settings
BROWSER_HTTP_PORT=8080 \
BROWSER_HEADLESS=1 \
BROWSER_API_KEY=your-secret-key \
BROWSER_DEFAULT_TIMEOUT_MS=60000 \
./target/release/agent-browser-http
```

#### Quick Test

```bash
# Health check
curl http://localhost:3000/health

# Navigate to a page
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'

# Get page snapshot
curl http://localhost:3000/snapshot | jq '.data.title'

# Take screenshot
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > screenshot.png
```

### Rust Library

Use Agent Browser directly in your Rust project.

#### Add Dependency

```toml
[dependencies]
agent-browser-core = "0.2"
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

#### Basic Usage

```rust
use agent_browser_core::{BrowserEngine, BrowserConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create browser engine (headed mode shows browser window)
    let engine = BrowserEngine::new(BrowserConfig::headed());

    // Navigate to a page
    engine.navigate("https://example.com").await?;

    // Get page snapshot
    let snapshot = engine.snapshot().await?;
    println!("Title: {}", snapshot.title);
    println!("Nodes: {}", snapshot.nodes.len());

    // Click element using ref_id
    engine.click("ax1").await?;

    // Or use CSS selector directly (recommended)
    engine.click_selector("button.submit", None).await?;

    // Type text
    engine.type_selector("input[name='email']", "hello@example.com", true, None).await?;

    // Take screenshot
    let screenshot = engine.screenshot().await?;

    // Shutdown
    engine.shutdown().await?;

    Ok(())
}
```

#### Anti-Detection Configuration

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

// New headless mode with stealth scripts
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)  // Chrome 112+ headless, harder to detect
    .with_stealth(true);                // Inject anti-detection scripts

let engine = BrowserEngine::new(config);
```

#### Custom Configuration

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_browser_path("/path/to/chrome")
    .with_profile_dir("/path/to/profile")  // Persist cookies
    .with_stealth(true)
    .with_arg("--disable-web-security");

let engine = BrowserEngine::new(config);
```

## Environment Variables

### HTTP Server

| Variable | Description | Default |
|----------|-------------|---------|
| `BROWSER_HTTP_HOST` | Bind address | `127.0.0.1` |
| `BROWSER_HTTP_PORT` | Server port | `3000` |
| `BROWSER_HEADLESS` | Browser display mode | new headless |
| `BROWSER_API_KEY` | API key for authentication | - |
| `BROWSER_DEFAULT_TIMEOUT_MS` | Default timeout in milliseconds | `30000` |
| `BROWSER_ALLOWED_FILE_ROOTS` | Upload/download roots | current directory and temp directory |

## Next Steps

- [API Reference](./api-reference.md) - Complete API documentation
- [Configuration](./configuration.md) - Detailed configuration options
- [Examples](./examples.md) - More usage examples
- [Architecture](./architecture.md) - System design
