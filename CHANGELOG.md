# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2025-04-07

### Added

#### MCP 2025-11-25 Protocol Support
- **Protocol Version Upgrade** - Upgraded from `2024-11-05` to `2025-11-25`
- **Version Negotiation** - Automatic protocol version negotiation with clients
- **Server Capabilities** - Full capability declarations:
  - `tools` - Tool support with `listChanged`
  - `resources` - Resource support with `subscribe` and `listChanged`
  - `prompts` - Prompt support with `listChanged`
  - `logging` - Logging capability

#### MCP Resources
- `resource://browser/screenshot` - Current page screenshot (PNG, base64)
- `resource://browser/snapshot` - Accessibility tree snapshot (text)

#### MCP Prompts
- `analyze_page` - Analyze page structure and content
- `fill_form` - Guide for filling out forms
- `extract_data` - Extract structured data from page

#### Tool Annotations
All tools now include behavior annotations:
- `readOnlyHint` - Tool only reads data, no side effects
- `destructiveHint` - Tool may cause irreversible changes
- `idempotentHint` - Same input always produces same result
- `openWorldHint` - Tool interacts with external systems

#### New Tools
- `browser_navigate_with_options` - Navigate with custom wait strategy
- `browser_download_file` - Download file from URL
- `browser_click_and_download` - Click element and wait for download
- `browser_press_key` - Press key with modifier keys (Ctrl, Alt, Shift, Cmd)
- `browser_shortcut` - Send predefined keyboard shortcuts
- `browser_enable_network_monitoring` - Enable network request capture
- `browser_get_network_requests` - Get captured network requests
- `browser_clear_network_requests` - Clear network request records
- `browser_enable_console_monitoring` - Enable console message capture
- `browser_get_console_messages` - Get captured console messages
- `browser_clear_console_messages` - Clear console message records
- `browser_set_viewport` - Set browser viewport size
- `browser_get_viewport` - Get current viewport size

#### Transport Layer
- **Modular Transport Architecture** - Separated transport implementations
- `transport::stdio` - STDIO transport (production ready)
- `transport::sse` - SSE transport (client implementation)
- `transport::http` - Streamable HTTP transport (client implementation)
- **Transport Trait** - Unified transport interface

#### Command Line
- `--transport <TYPE>` - Select transport type (stdio, sse, http)
- `--port <PORT>` - Port for HTTP/SSE transport
- `--help` - Show help message

### Changed

#### Protocol Types
- Complete rewrite of `protocol.rs` with full MCP 2025-11-25 types
- Added `InitializeParams`, `InitializeResult`, `ClientCapabilities`, `ServerCapabilities`
- Added `Tool`, `ToolAnnotations`, `ToolsListResult`, `ToolCallResult`
- Added `Resource`, `ResourcesListResult`, `ResourceReadResult`, `ResourceContents`
- Added `Prompt`, `PromptArgument`, `PromptsListResult`, `PromptGetResult`
- Added `Content`, `ResourceLink` for content blocks
- Added `ProgressParams` for progress tracking
- Added `LogLevel`, `SetLogLevelParams`, `LoggingMessageParams`

#### Server Implementation
- Refactored `main.rs` with modular request handlers
- Added `handle_initialize` with version negotiation
- Added `handle_tools_list`, `handle_tools_call`
- Added `handle_resources_list`, `handle_resources_read`
- Added `handle_prompts_list`, `handle_prompts_get`
- Added `handle_set_log_level`
- Added notification handling (`notifications/initialized`, `notifications/cancelled`)

### Fixed
- Proper JSON-RPC 2.0 message parsing
- Correct response format with `jsonrpc: "2.0"` as string

### Technical
- Added `base64` dependency for screenshot encoding
- Added `futures` dependency for stream handling
- Added `reqwest` with `json` and `stream` features
- Added `async-trait` for transport trait
- All 4 unit tests passing

---

## [0.1.0] - 2025-03-17

### Added

#### Core Features
- **BrowserEngine** - Core browser automation engine based on CDP
- **Accessibility Tree Support** - Semantic element location using A11y tree
- **Multi-tab Management** - Create, switch, and close browser tabs
- **Iframe Support** - Enter/exit iframe contexts

#### MCP Server
- 17 MCP tools for AI agents
- JSON-RPC 2.0 protocol implementation
- Integration with Claude Code, Cursor, and other MCP clients

#### HTTP API
- RESTful API for browser control
- WebSocket support for real-time events
- CORS support for web clients

#### Browser Control
- Navigation with wait options
- Element actions (click, type, scroll, drag)
- Screenshot (full page, element, viewport)
- Cookie management
- JavaScript evaluation
- File upload
- Dialog handling
- Download support

#### Advanced Features
- **New Headless Mode** - `--headless=new` for better anti-detection
- **Stealth Mode** - Inject scripts to hide automation detection
- **CSS Selector Operations** - Direct element operations without ref_id
  - `click_selector` - Click by CSS selector
  - `type_selector` - Type by CSS selector
  - `get_text` - Get element text
  - `get_attribute` - Get element attribute
  - `element_exists` - Check element existence
  - `hover_selector` - Mouse hover
  - `select_option` - Select dropdown option
  - `expand_and_click_submenu` - Handle submenu interactions

#### Configuration
- Automatic Chrome/Chromium detection
- Custom browser path support
- Profile directory for cookie persistence
- Configurable timeouts
- Extra Chrome arguments

### Technical
- Built with Rust 1.75+
- Uses chromiumoxide 0.9 for CDP
- Async runtime with Tokio
- HTTP server with Axum

---

## Future Plans

- [ ] PDF generation
- [ ] Network interception
- [ ] Performance metrics
- [ ] Multi-browser support (Firefox, Safari)
- [ ] Visual regression testing
- [ ] Recording and playback