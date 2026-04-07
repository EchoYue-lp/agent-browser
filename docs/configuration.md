# Configuration

Detailed configuration options for Agent Browser.

## BrowserConfig

The main configuration struct for browser settings.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `headless` | `HeadlessMode` | `New` | Headless browser mode |
| `browser_path` | `Option<PathBuf>` | Auto-detect | Path to Chrome/Chromium executable |
| `profile_dir` | `Option<PathBuf>` | `None` | User data directory for cookie persistence |
| `navigation_timeout_ms` | `u64` | `30000` | Navigation timeout in milliseconds |
| `action_timeout_ms` | `u64` | `10000` | Action timeout in milliseconds |
| `stealth` | `bool` | `true` | Enable anti-detection scripts |
| `extra_args` | `Vec<String>` | `[]` | Additional Chrome arguments |

### HeadlessMode

```rust
pub enum HeadlessMode {
    /// Visible browser window
    None,
    /// Old headless mode (detectable)
    Old,
    /// New headless mode (Chrome 112+, harder to detect)
    New,
}
```

## Builder Methods

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

let config = BrowserConfig::default()
    // Headless mode
    .with_headless(HeadlessMode::New)

    // Custom browser path
    .with_browser_path("/usr/bin/google-chrome")

    // Persist cookies and session
    .with_profile_dir("~/.config/chrome-profile")

    // Enable/disable stealth
    .with_stealth(true)

    // Add Chrome arguments
    .with_arg("--disable-web-security")
    .with_arg("--window-size=1920,1080");
```

## Presets

### Headed Mode (Visible Browser)

```rust
let config = BrowserConfig::headed();
```

### Headless Mode (New)

```rust
let config = BrowserConfig::headless();
```

### Headless Mode (Old)

```rust
let config = BrowserConfig::headless_old();
```

## Environment Variables

### HTTP Server

| Variable | Description | Default |
|----------|-------------|---------|
| `BROWSER_HTTP_PORT` | Server port | `3000` |
| `BROWSER_HEADLESS` | Enable headless mode (any value) | - |
| `BROWSER_API_KEY` | API key for authentication | - |
| `BROWSER_DEFAULT_TIMEOUT_MS` | Default timeout in milliseconds | `30000` |

### Example

```bash
# Start HTTP server with custom settings
BROWSER_HTTP_PORT=8080 \
BROWSER_HEADLESS=1 \
BROWSER_API_KEY=secret123 \
BROWSER_DEFAULT_TIMEOUT_MS=60000 \
./target/release/agent-browser-http
```

## Chrome Arguments

### Useful Arguments

| Argument | Description |
|----------|-------------|
| `--disable-web-security` | Disable same-origin policy |
| `--disable-features=IsolateOrigins,site-per-process` | Disable site isolation |
| `--window-size=WIDTH,HEIGHT` | Set window size |
| `--disable-gpu` | Disable GPU hardware acceleration |
| `--no-sandbox` | Disable sandbox (required for some environments) |
| `--disable-setuid-sandbox` | Disable setuid sandbox |
| `--disable-dev-shm-usage` | Use /tmp instead of /dev/shm |
| `--disable-blink-features=AutomationControlled` | Hide automation indicators |

### Adding Arguments

```rust
let config = BrowserConfig::default()
    .with_arg("--disable-web-security")
    .with_arg("--window-size=1920,1080");
```

## Anti-Detection

### Stealth Mode

When `stealth: true`, Agent Browser injects JavaScript to:

1. Hide `navigator.webdriver` property
2. Modify `navigator.plugins` to appear normal
3. Override `navigator.languages`
4. Hide Chrome automation indicators

### New Headless Mode

Chrome 112+ introduces a new headless mode that:

- Shares the same browser code as headed mode
- Is harder to detect than old headless mode
- Recommended for production scraping

```rust
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_stealth(true);
```

## Cookie Persistence

Use `profile_dir` to persist cookies and session data:

```rust
let config = BrowserConfig::default()
    .with_profile_dir("/path/to/profile");
```

This allows:
- Staying logged in across sessions
- Persisting cookies
- Maintaining local storage data

## Logging

### Enable Debug Logging

```bash
# For HTTP server
RUST_LOG=agent_browser_http=debug,agent_browser_core=debug \
./target/release/agent-browser-http

# For MCP server
RUST_LOG=agent_browser_mcp=debug,agent_browser_core=debug \
./target/release/agent-browser-mcp
```

### Log Levels

| Level | Description |
|-------|-------------|
| `error` | Only errors |
| `warn` | Warnings and errors |
| `info` | General information (default) |
| `debug` | Detailed debug information |
| `trace` | Very verbose output |

### MCP Logging

The MCP server supports `logging/setLevel` to configure log levels via the protocol:

```json
{
  "method": "logging/setLevel",
  "params": {
    "level": "debug"
  }
}
```

Supported levels: `debug`, `info`, `notice`, `warning`, `error`, `critical`, `alert`, `emergency`

## Platform-Specific Notes

### macOS

Chrome is typically installed at:
```
/Applications/Google Chrome.app/Contents/MacOS/Google Chrome
```

Auto-detection is supported.

### Linux

Chrome paths checked:
```
/usr/bin/google-chrome
/usr/bin/chromium
/usr/bin/chromium-browser
```

### Windows

Chrome paths checked:
```
C:\Program Files\Google\Chrome\Application\chrome.exe
C:\Program Files (x86)\Google\Chrome\Application\chrome.exe
```

## Examples

### Production Scraping

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_stealth(true)
    .with_arg("--disable-blink-features=AutomationControlled")
    .with_arg("--window-size=1920,1080");
```

### Development/Debugging

```rust
let config = BrowserConfig::headed()
    .with_stealth(false);
```

### Docker Environment

```rust
let config = BrowserConfig::headless()
    .with_stealth(true)
    .with_arg("--no-sandbox")
    .with_arg("--disable-setuid-sandbox")
    .with_arg("--disable-dev-shm-usage");
```