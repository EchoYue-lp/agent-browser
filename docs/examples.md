# Examples

Practical examples demonstrating Agent Browser capabilities.

## Table of Contents

- [Web Scraping](#web-scraping)
- [Form Filling](#form-filling)
- [CSS Selector Operations](#css-selector-operations)
- [Dynamic Content](#dynamic-content)
- [Authentication](#authentication)
- [File Operations](#file-operations)
- [Multi-Tab Operations](#multi-tab-operations)

## Web Scraping

### Basic Scraping

```bash
# Navigate to page
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://news.ycombinator.com"}'

# Get snapshot to understand page structure
curl http://localhost:3000/snapshot | jq '.data.nodes[] | select(.role == "link") | {name, ref_id}' | head -20

# Extract data using JavaScript
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "Array.from(document.querySelectorAll(\".titleline > a\")).map(a => ({title: a.textContent, href: a.href})).slice(0, 10)"}'
```

### Extract PDF Links

```bash
# Navigate to page with PDF links
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/documents"}'

# Find all PDF links
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "Array.from(document.querySelectorAll(\"a[href$=\\\".pdf\\\"]\")).map(a => ({text: a.textContent.trim(), href: a.href}))"}'
```

### Rust Example

```rust
use agent_browser_core::{BrowserEngine, BrowserConfig, HeadlessMode};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let engine = BrowserEngine::new(
        BrowserConfig::default()
            .with_headless(HeadlessMode::New)
            .with_stealth(true)
    )?;

    engine.launch().await?;
    engine.navigate("https://news.ycombinator.com").await?;

    // Get snapshot
    let snapshot = engine.snapshot().await?;
    println!("Found {} nodes", snapshot.nodes.len());

    // Extract links using JavaScript
    let links: Vec<serde_json::Value> = engine.evaluate(r#"
        Array.from(document.querySelectorAll('.titleline > a'))
            .map(a => ({title: a.textContent, href: a.href}))
            .slice(0, 10)
    "#).await?;

    for link in links {
        println!("- {}", link["title"]);
    }

    engine.shutdown().await?;
    Ok(())
}
```

## Form Filling

### Login Form

```bash
# Navigate to login page
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/login"}'

# Fill username
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"username\"]", "text": "myuser", "clear_first": true}'

# Fill password
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"password\"]", "text": "mypassword", "clear_first": true}'

# Submit form
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "button[type=\"submit\"]"}'

# Wait for navigation
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"selector": ".dashboard", "timeout_ms": 5000}'
```

### Complex Form with Dropdowns

```bash
# Fill text fields
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "#name", "text": "John Doe"}'

# Select from dropdown by value
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector": "#country", "value": "us"}'

# Select from dropdown by text
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector": "#city", "value": "New York", "by_text": true}'

# Check checkbox (click it)
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"agree\"]"}'
```

## CSS Selector Operations

### Direct Element Access

```bash
# Click by selector
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "button.primary"}'

# Get text content
curl -X POST http://localhost:3000/get-text \
  -H "Content-Type: application/json" \
  -d '{"selector": ".article-title"}'

# Get attribute
curl -X POST http://localhost:3000/get-attribute \
  -H "Content-Type: application/json" \
  -d '{"selector": "a.download", "attribute": "href"}'

# Check existence
curl -X POST http://localhost:3000/element-exists \
  -H "Content-Type: application/json" \
  -d '{"selector": ".error-message"}'

# Hover
curl -X POST http://localhost:3000/hover \
  -H "Content-Type: application/json" \
  -d '{"selector": ".menu-trigger"}'
```

### Handling Vue.js/React Components

```bash
# Many SPA frameworks use custom components
# Access them through their underlying structure

# Click a Vue component button
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "document.querySelector(\".el-menu-item:contains(\\\"Settings\\\")\").click()"}'

# Or find by text content
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "Array.from(document.querySelectorAll(\"button\")).find(b => b.textContent.includes(\"Submit\")).click()"}'
```

## Dynamic Content

### Wait for Elements

```bash
# Wait for selector to appear
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"selector": ".loaded-content", "timeout_ms": 10000}'

# Wait for network idle
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"idle_duration_ms": 1000, "timeout_ms": 30000}'

# Simple wait
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"timeout_ms": 2000}'
```

### Scroll and Load More

```bash
# Scroll down
curl -X POST http://localhost:3000/act \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax1", "action": "scroll", "direction": "down", "amount": 500}'

# Or use JavaScript for infinite scroll
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "window.scrollTo(0, document.body.scrollHeight)"}'
```

## Authentication

### Cookie-Based Auth

```bash
# Set authentication cookies
curl -X POST http://localhost:3000/cookies \
  -H "Content-Type: application/json" \
  -d '{
    "cookies": [
      {"name": "session_id", "value": "abc123", "domain": "example.com"}
    ]
  }'

# Navigate to protected page
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/dashboard"}'
```

### Persistent Session

```bash
# Use profile directory to persist cookies
BROWSER_PROFILE_DIR=/path/to/profile ./target/release/agent-browser-http
```

Or in Rust:

```rust
let config = BrowserConfig::default()
    .with_profile_dir("/path/to/profile");
```

## File Operations

### Download File

```bash
# Download by URL
curl -X POST http://localhost:3000/download \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.pdf", "save_path": "/downloads"}'

# Click and download
curl -X POST http://localhost:3000/click-download \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax10", "save_path": "/downloads", "timeout_ms": 60000}'
```

### File Upload

```bash
# Upload to file input
curl -X POST http://localhost:3000/upload \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax5", "file_path": "/path/to/document.pdf"}'
```

## Multi-Tab Operations

### Working with Multiple Tabs

```bash
# List all tabs
curl http://localhost:3000/tabs

# Switch to specific tab
curl -X POST http://localhost:3000/tabs/TAB_ID/activate

# Close tab
curl -X DELETE http://localhost:3000/tabs/TAB_ID
```

### Rust Example

```rust
// List and switch tabs
let tabs = engine.list_tabs().await?;
for tab in &tabs {
    println!("Tab: {} - {}", tab.tab_id, tab.title);
}

// Switch to first tab
if !tabs.is_empty() {
    engine.activate_tab(&tabs[0].tab_id).await?;
}

// Close other tabs
for tab in tabs.iter().skip(1) {
    engine.close_tab(&tab.tab_id).await?;
}
```

## Screenshots

### Various Screenshot Options

```bash
# Viewport screenshot
curl http://localhost:3000/screenshot | jq -r '.data.image' | base64 -d > viewport.png

# Full page screenshot
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > fullpage.png

# Element screenshot
curl "http://localhost:3000/screenshot?selector=.main-content" | jq -r '.data.image' | base64 -d > element.png
```

### Rust Example

```rust
// Take screenshot
let screenshot = engine.screenshot(Some(ScreenshotOptions {
    full_page: Some(true),
    selector: None,
})).await?;

std::fs::write("screenshot.png", base64::decode(&screenshot.data)?)?;
```

## Error Handling

### Robust Scraping

```rust
use agent_browser_core::{BrowserEngine, BrowserConfig, Error};

async fn robust_click(engine: &BrowserEngine, selector: &str) -> Result<(), Error> {
    // Wait for element first
    match engine.wait_for_selector(selector, 5000).await {
        Ok(_) => {},
        Err(_) => {
            // Try scrolling if not found
            engine.evaluate("window.scrollBy(0, 500)").await?;
            engine.wait_for_selector(selector, 5000).await?;
        }
    }

    // Click with retry
    for attempt in 0..3 {
        match engine.click_selector(selector, None).await {
            Ok(_) => return Ok(()),
            Err(e) if attempt < 2 => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}
```