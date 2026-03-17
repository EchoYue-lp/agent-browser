# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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