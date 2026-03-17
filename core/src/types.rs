//! Public type definitions for browser automation.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Headless mode configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadlessMode {
    /// No headless mode (visible browser window).
    None,
    /// Old headless mode (easier to detect).
    Old,
    /// New headless mode (Chrome 112+, harder to detect).
    #[default]
    New,
}

/// Browser configuration.
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    /// Headless mode setting.
    pub headless: HeadlessMode,
    /// Browser executable path.
    pub browser_path: Option<PathBuf>,
    /// User data directory (for cookie persistence).
    pub profile_dir: Option<PathBuf>,
    /// Default navigation timeout in milliseconds.
    pub navigation_timeout_ms: u64,
    /// Default action timeout in milliseconds.
    pub action_timeout_ms: u64,
    /// Enable anti-detection scripts.
    pub stealth: bool,
    /// Extra browser launch arguments.
    pub extra_args: Vec<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        // Auto-detect Chrome path
        let browser_path = Self::detect_chrome_path();

        Self {
            headless: HeadlessMode::New,
            browser_path,
            profile_dir: None,
            navigation_timeout_ms: 30_000,
            action_timeout_ms: 10_000,
            stealth: true,
            extra_args: Vec::new(),
        }
    }
}

impl BrowserConfig {
    /// Create a headless configuration (new headless mode).
    pub fn headless() -> Self {
        Self {
            headless: HeadlessMode::New,
            ..Default::default()
        }
    }

    /// Create a headed configuration (visible browser window).
    pub fn headed() -> Self {
        Self {
            headless: HeadlessMode::None,
            ..Default::default()
        }
    }

    /// Create an old headless configuration (better compatibility, easier to detect).
    pub fn headless_old() -> Self {
        Self {
            headless: HeadlessMode::Old,
            ..Default::default()
        }
    }

    /// Set the browser executable path.
    pub fn with_browser_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.browser_path = Some(path.into());
        self
    }

    /// Set the user data directory.
    pub fn with_profile_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.profile_dir = Some(dir.into());
        self
    }

    /// Set the headless mode.
    pub fn with_headless(mut self, mode: HeadlessMode) -> Self {
        self.headless = mode;
        self
    }

    /// Enable or disable stealth mode.
    pub fn with_stealth(mut self, stealth: bool) -> Self {
        self.stealth = stealth;
        self
    }

    /// Add a browser launch argument.
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Auto-detect Chrome path on the system.
    fn detect_chrome_path() -> Option<PathBuf> {
        // macOS
        #[cfg(target_os = "macos")]
        let macos_paths = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];

        // Linux
        #[cfg(target_os = "linux")]
        let linux_paths = [
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
        ];

        // Windows (常见的安装路径)
        #[cfg(windows)]
        let windows_paths = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];

        #[cfg(target_os = "macos")]
        {
            for path in &macos_paths {
                if std::path::Path::new(path).exists() {
                    return Some(PathBuf::from(path));
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            for path in &linux_paths {
                if std::path::Path::new(path).exists() {
                    return Some(PathBuf::from(path));
                }
            }
        }

        #[cfg(windows)]
        {
            for path in &windows_paths {
                if std::path::Path::new(path).exists() {
                    return Some(PathBuf::from(path));
                }
            }
        }

        None
    }
}

/// Element bounding box.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Page information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    /// Current URL.
    pub url: String,
    /// Page title.
    pub title: String,
}

/// Screenshot result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    /// Base64 encoded image data.
    pub data: String,
    /// Image format (png/jpeg).
    pub format: String,
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
}

/// Cookie information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieInfo {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: bool,
    pub http_only: bool,
}

/// Cookie parameter for setting cookies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetCookieParam {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: Option<bool>,
    pub http_only: Option<bool>,
}

/// Tab information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub tab_id: String,
    pub url: String,
    pub title: String,
    pub active: bool,
}

/// Screenshot options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotOptions {
    /// Whether to capture the full page.
    pub full_page: Option<bool>,
    /// CSS selector to capture a specific element.
    pub selector: Option<String>,
}

/// Wait options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitOptions {
    /// CSS selector to wait for.
    pub selector: Option<String>,
    /// Timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// Download options.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DownloadOptions {
    /// Download save directory (defaults to temp directory).
    #[serde(default)]
    pub save_path: Option<String>,
    /// Download timeout in milliseconds (default 60000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Download result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResult {
    /// Download GUID.
    pub guid: String,
    /// File name.
    pub filename: String,
    /// Full file path.
    pub file_path: String,
    /// File size in bytes.
    pub size: Option<u64>,
    /// MIME type.
    pub mime_type: Option<String>,
    /// Download status.
    pub status: DownloadStatus,
}

/// Download status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    InProgress,
    Completed,
    Canceled,
    Interrupted,
}

/// Keyboard modifier keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyModifier {
    Alt,
    Control,
    Meta, // Command on Mac
    Shift,
}

/// Key press options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressOptions {
    /// The key to press.
    pub key: String,
    /// Modifier keys.
    #[serde(default)]
    pub modifiers: Vec<KeyModifier>,
}

/// Navigation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigateResult {
    /// Page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Final URL (after redirects).
    pub final_url: String,
}

/// Generic tool execution result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Output content.
    pub output: String,
    /// Error message (when failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            error: None,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error.into()),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headless_mode_default() {
        assert_eq!(HeadlessMode::default(), HeadlessMode::New);
    }

    #[test]
    fn test_browser_config_default() {
        let config = BrowserConfig::default();
        assert_eq!(config.headless, HeadlessMode::New);
        assert!(config.stealth);
        assert_eq!(config.navigation_timeout_ms, 30_000);
        assert_eq!(config.action_timeout_ms, 10_000);
        assert!(config.extra_args.is_empty());
    }

    #[test]
    fn test_browser_config_headless() {
        let config = BrowserConfig::headless();
        assert_eq!(config.headless, HeadlessMode::New);
    }

    #[test]
    fn test_browser_config_headed() {
        let config = BrowserConfig::headed();
        assert_eq!(config.headless, HeadlessMode::None);
    }

    #[test]
    fn test_browser_config_headless_old() {
        let config = BrowserConfig::headless_old();
        assert_eq!(config.headless, HeadlessMode::Old);
    }

    #[test]
    fn test_browser_config_builder() {
        let config = BrowserConfig::default()
            .with_headless(HeadlessMode::None)
            .with_stealth(false)
            .with_browser_path("/usr/bin/chrome")
            .with_profile_dir("/tmp/profile")
            .with_arg("--disable-web-security");

        assert_eq!(config.headless, HeadlessMode::None);
        assert!(!config.stealth);
        assert_eq!(config.browser_path, Some(PathBuf::from("/usr/bin/chrome")));
        assert_eq!(config.profile_dir, Some(PathBuf::from("/tmp/profile")));
        assert!(
            config
                .extra_args
                .contains(&"--disable-web-security".to_string())
        );
    }

    #[test]
    fn test_bounds_serialization() {
        let bounds = Bounds {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 200.0,
        };
        let json = serde_json::to_string(&bounds).unwrap();
        assert!(json.contains("\"x\":10.0"));
        assert!(json.contains("\"y\":20.0"));

        let parsed: Bounds = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.x, 10.0);
        assert_eq!(parsed.width, 100.0);
    }

    #[test]
    fn test_cookie_info_serialization() {
        let cookie = CookieInfo {
            name: "session".to_string(),
            value: "abc123".to_string(),
            domain: Some("example.com".to_string()),
            path: Some("/".to_string()),
            secure: true,
            http_only: false,
        };

        let json = serde_json::to_string(&cookie).unwrap();
        let parsed: CookieInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "session");
        assert_eq!(parsed.value, "abc123");
        assert!(parsed.secure);
    }

    #[test]
    fn test_tab_info_serialization() {
        let tab = TabInfo {
            tab_id: "tab-123".to_string(),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            active: true,
        };

        let json = serde_json::to_string(&tab).unwrap();
        let parsed: TabInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tab_id, "tab-123");
        assert!(parsed.active);
    }

    #[test]
    fn test_download_status_serialization() {
        let status = DownloadStatus::Completed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"completed\"");

        let parsed: DownloadStatus = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DownloadStatus::Completed));
    }

    #[test]
    fn test_key_modifier_serialization() {
        let modifier = KeyModifier::Control;
        let json = serde_json::to_string(&modifier).unwrap();
        assert_eq!(json, "\"control\"");

        let parsed: KeyModifier = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, KeyModifier::Control));
    }

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::success("Operation completed");
        assert!(result.success);
        assert_eq!(result.output, "Operation completed");
        assert!(result.error.is_none());
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("Something went wrong");
        assert!(!result.success);
        assert!(result.output.is_empty());
        assert_eq!(result.error, Some("Something went wrong".to_string()));
    }

    #[test]
    fn test_tool_result_serialization() {
        let result = ToolResult::success("test output");
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"output\":\"test output\""));
        // error should be skipped when None
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_screenshot_options_default() {
        let options = ScreenshotOptions {
            full_page: None,
            selector: None,
        };
        let json = serde_json::to_string(&options).unwrap();
        let parsed: ScreenshotOptions = serde_json::from_str(&json).unwrap();
        assert!(parsed.full_page.is_none());
        assert!(parsed.selector.is_none());
    }

    #[test]
    fn test_navigate_result() {
        let result = NavigateResult {
            url: "https://example.com".to_string(),
            title: "Example Domain".to_string(),
            final_url: "https://example.com/".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: NavigateResult = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.url, "https://example.com");
        assert_eq!(parsed.title, "Example Domain");
    }
}
