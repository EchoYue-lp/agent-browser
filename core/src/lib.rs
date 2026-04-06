//! # Agent Browser Core
//!
//! A high-performance browser automation library designed for AI agents.
//!
//! Built on Chrome DevTools Protocol (CDP) for reliable browser control.
//!
//! ## Core Features
//!
//! - **Semantic Element Location**: Based on Accessibility Tree, more stable and reliable
//! - **Smart State Management**: Automatically maintains active page, no need to track tab_id
//! - **Anti-Detection**: Supports new headless mode and Stealth scripts
//! - **CSS Selector Operations**: Direct element operations without ref_id
//! - **Iframe Support**: Complete iframe context switching capability
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                 Agent Application                    │
//! └──────────────────────────┬──────────────────────────┘
//!                            │
//!                            ▼
//! ┌─────────────────────────────────────────────────────┐
//! │                 BrowserEngine                        │
//! │  - Lifecycle management (launch, shutdown)           │
//! │  - Page navigation (navigate, snapshot)              │
//! │  - Element actions (click, type, scroll)             │
//! │  - CSS selector operations (click_selector, etc.)    │
//! └──────────────────────────┬──────────────────────────┘
//!                            │ CDP
//!                            ▼
//!                     Chrome/Chromium
//! ```
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use agent_browser_core::{BrowserEngine, BrowserConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create browser engine (headed mode, shows browser window)
//!     let engine = BrowserEngine::new(BrowserConfig::headed());
//!
//!     // Navigate to page
//!     engine.navigate("https://example.com").await?;
//!
//!     // Get page snapshot (Accessibility Tree)
//!     let snapshot = engine.snapshot().await?;
//!     println!("Page title: {}", snapshot.title);
//!     println!("Element count: {}", snapshot.nodes.len());
//!
//!     // Method 1: Click element using ref_id
//!     engine.click("ax1").await?;
//!
//!     // Method 2: Use CSS selector directly (recommended)
//!     engine.click_selector("button.submit", None).await?;
//!     engine.type_selector("input[name='email']", "hello@example.com", false, None).await?;
//!
//!     // Screenshot
//!     let screenshot = engine.screenshot().await?;
//!
//!     // Close browser
//!     engine.shutdown().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Anti-detection Configuration
//!
//! ```rust
//! use agent_browser_core::{BrowserConfig, HeadlessMode};
//!
//! // New headless mode + anti-detection scripts
//! let config = BrowserConfig::default()
//!     .with_headless(HeadlessMode::New)  // New headless mode, harder to detect
//!     .with_stealth(true);                // Inject anti-detection scripts
//!
//! # fn main() {}
//! ```
//!
//! ## CSS Selector Operations
//!
//! No need to get snapshot first, operate elements directly using CSS selectors:
//!
//! ```rust,no_run
//! # use agent_browser_core::{BrowserEngine, BrowserConfig};
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! # let engine = BrowserEngine::new(BrowserConfig::default());
//!
//! // Click
//! engine.click_selector("button.primary", None).await?;
//!
//! // Type text
//! engine.type_selector("input#username", "myuser", true, None).await?;
//!
//! // Get text content
//! let text = engine.get_text(".article-title", None).await?;
//!
//! // Check if element exists
//! let exists = engine.element_exists(".login-form").await?;
//!
//! // Select dropdown option
//! engine.select_option("select#country", "china", false, None).await?;
//!
//! // Mouse hover
//! engine.hover_selector(".dropdown-trigger", None).await?;
//!
//! // Expand menu and click submenu
//! engine.expand_and_click_submenu(".menu-item", ".submenu .action", None).await?;
//!
//! # Ok(())
//! # }
//! ```

pub mod actions;
pub mod browser;
pub mod error;
pub mod snapshot;
pub mod types;

pub use actions::{ActionKind, ActionResult};
pub use browser::{BrowserEngine, BrowserHandle, IframeContext};
pub use error::{Error, Result};
pub use snapshot::{PageSnapshot, SnapshotNode};
pub use types::{
    Bounds, BrowserConfig, ConsoleMessage, CookieInfo, DownloadOptions, DownloadResult,
    DownloadStatus, HeadlessMode, KeyModifier, NavigateResult, NavigationWaitUntil,
    NetworkRequest, NetworkResponse, PageInfo, PressOptions, ScreenshotOptions, ScreenshotResult,
    SetCookieParam, TabInfo, ToolResult, ViewportSize, WaitOptions,
};
