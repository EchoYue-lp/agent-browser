//! Browser lifecycle management.
//!
//! CDP connection management based on chromiumoxide, providing:
//! - Browser launch/shutdown
//! - Page navigation
//! - State management
//! - Cookie management
//! - Multi-tab management
//! - File upload/download
//! - Network idle detection
//! - iframe context switching
//! - Keyboard shortcuts

use chromiumoxide::{Browser, Page, browser::BrowserConfig as ChromeConfig};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, info, warn};
use url::Url;

use crate::actions::{ActionKind, ActionResult};
use crate::error::{Error, Result};
use crate::snapshot;
use crate::snapshot::{PageSnapshot, SnapshotDiff, SnapshotOptions, SnapshotSearchResult};
use crate::types::{
    BrowserConfig, BrowserEvent, CookieInfo, DownloadOptions, DownloadResult, DownloadStatus,
    HeadlessMode, KeyModifier, NavigationWaitUntil, ScreenshotOptions, SetCookieParam, TabInfo,
};

/// Validate a file path for security.
///
/// Checks for:
/// - Path traversal attempts (..)
/// - Absolute path requirements
/// - Symlink resolution
fn validate_file_path(path: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    let canonical = resolve_path(Path::new(path), false)?;
    if !canonical.is_file() {
        return Err(Error::InvalidPath(format!(
            "Upload path is not a regular file: {}",
            canonical.display()
        )));
    }
    ensure_path_allowed(&canonical, allowed_roots)?;
    Ok(canonical)
}

/// Validate a directory path for security.
fn validate_directory_path(path: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    let resolved = resolve_path(Path::new(path), true)?;
    ensure_path_allowed(&resolved, allowed_roots)?;
    Ok(resolved)
}

fn resolve_path(path: &Path, allow_missing: bool) -> Result<PathBuf> {
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(Error::InvalidPath(
            "Parent-directory components are not allowed".to_string(),
        ));
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(Error::Io)?.join(path)
    };

    if absolute.exists() {
        return absolute
            .canonicalize()
            .map_err(|e| Error::InvalidPath(format!("Failed to resolve path: {e}")));
    }

    if !allow_missing {
        return Err(Error::InvalidPath(format!(
            "Path does not exist: {}",
            absolute.display()
        )));
    }

    let mut ancestor = absolute.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| {
            Error::InvalidPath(format!("No existing parent for {}", absolute.display()))
        })?;
    }

    let canonical_ancestor = ancestor
        .canonicalize()
        .map_err(|e| Error::InvalidPath(format!("Failed to resolve parent path: {e}")))?;
    let suffix = absolute
        .strip_prefix(ancestor)
        .map_err(|e| Error::InvalidPath(format!("Failed to normalize path: {e}")))?;
    Ok(canonical_ancestor.join(suffix))
}

fn ensure_path_allowed(path: &Path, allowed_roots: &[PathBuf]) -> Result<()> {
    let allowed = allowed_roots.iter().any(|root| {
        resolve_path(root, true)
            .map(|resolved_root| path.starts_with(resolved_root))
            .unwrap_or(false)
    });

    if allowed {
        Ok(())
    } else {
        Err(Error::InvalidPath(format!(
            "Path is outside configured allowed roots: {}",
            path.display()
        )))
    }
}

fn origin_matches(pattern: &str, url: &Url) -> bool {
    let pattern = pattern.trim().trim_end_matches('/');
    if pattern == "*" {
        return true;
    }

    let origin = url.origin().ascii_serialization();
    if pattern.eq_ignore_ascii_case(&origin) {
        return true;
    }

    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let (pattern_scheme, authority) = pattern
        .split_once("://")
        .map_or((None, pattern), |(scheme, authority)| {
            (Some(scheme), authority)
        });
    if pattern_scheme.is_some_and(|scheme| !scheme.eq_ignore_ascii_case(url.scheme())) {
        return false;
    }

    let authority = authority.split('/').next().unwrap_or(authority);
    let (pattern_host, pattern_port) = authority
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host, Some(port))))
        .unwrap_or((authority, None));
    if let Some(port) = pattern_port {
        if url.port_or_known_default() != Some(port) {
            return false;
        }
    } else if pattern_scheme.is_some()
        && url.port().is_some()
        && url.port_or_known_default()
            != match url.scheme() {
                "http" | "ws" => Some(80),
                "https" | "wss" => Some(443),
                _ => None,
            }
    {
        return false;
    }
    let pattern_host = pattern_host.to_ascii_lowercase();

    pattern_host
        .strip_prefix("*.")
        .is_some_and(|suffix| host.ends_with(&format!(".{suffix}")))
}

fn is_restricted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || matches!(ip.octets(), [100, second, _, _] if (64..=127).contains(&second))
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
        }
    }
}

async fn validate_url_policy(
    config: &BrowserConfig,
    cache: &Mutex<HashMap<String, (bool, Instant)>>,
    raw_url: &str,
) -> Result<Url> {
    let url = Url::parse(raw_url)
        .map_err(|error| Error::InvalidParameter(format!("Invalid URL: {error}")))?;
    if matches!(url.scheme(), "about" | "blob" | "data") {
        return Ok(url);
    }
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return Err(Error::NetworkAccessDenied(format!(
            "Unsupported URL scheme: {}",
            url.scheme()
        )));
    }

    if config
        .blocked_origins
        .iter()
        .any(|pattern| origin_matches(pattern, &url))
    {
        return Err(Error::NetworkAccessDenied(format!(
            "Origin is blocked: {}",
            url.origin().ascii_serialization()
        )));
    }
    if !config.allowed_origins.is_empty()
        && !config
            .allowed_origins
            .iter()
            .any(|pattern| origin_matches(pattern, &url))
    {
        return Err(Error::NetworkAccessDenied(format!(
            "Origin is not allowlisted: {}",
            url.origin().ascii_serialization()
        )));
    }

    if config.allow_private_networks {
        return Ok(url);
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::NetworkAccessDenied("URL has no host".to_string()))?;
    if let Some((allowed, checked_at)) = cache.lock().await.get(host).copied()
        && checked_at.elapsed() < Duration::from_secs(5)
    {
        return if allowed {
            Ok(url)
        } else {
            Err(Error::NetworkAccessDenied(format!(
                "Host resolves to a private or local network: {host}"
            )))
        };
    }

    let allowed = if let Ok(ip) = host.parse::<IpAddr>() {
        !is_restricted_ip(ip)
    } else {
        let port = url.port_or_known_default().unwrap_or(443);
        let resolved = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| {
                Error::NetworkAccessDenied(format!("Failed to resolve host {host}: {error}"))
            })?
            .collect::<Vec<_>>();
        !resolved.is_empty() && resolved.iter().all(|addr| !is_restricted_ip(addr.ip()))
    };
    cache
        .lock()
        .await
        .insert(host.to_string(), (allowed, Instant::now()));

    if allowed {
        Ok(url)
    } else {
        Err(Error::NetworkAccessDenied(format!(
            "Host resolves to a private or local network: {host}"
        )))
    }
}

fn sanitize_headers(
    mut headers: serde_json::Value,
    capture_sensitive_data: bool,
) -> serde_json::Value {
    if capture_sensitive_data {
        return headers;
    }
    if let Some(object) = headers.as_object_mut() {
        for (name, value) in object {
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization"
                    | "proxy-authorization"
                    | "cookie"
                    | "set-cookie"
                    | "x-api-key"
                    | "x-auth-token"
            ) {
                *value = serde_json::Value::String("<redacted>".to_string());
            }
        }
    }
    headers
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let mut remaining = value;
    let mut first = true;
    for part in pattern.split('*') {
        if part.is_empty() {
            continue;
        }
        let Some(index) = remaining.find(part) else {
            return false;
        };
        if first && !pattern.starts_with('*') && index != 0 {
            return false;
        }
        remaining = &remaining[index + part.len()..];
        first = false;
    }
    pattern.ends_with('*') || remaining.is_empty()
}

/// Browser handle.
///
/// Lightweight, cloneable browser operation handle.
/// All Browser methods accept `&self`, supporting concurrent operations.
#[derive(Clone)]
pub struct BrowserHandle(Arc<Browser>);

impl BrowserHandle {
    /// Create a new tab and navigate to URL.
    pub async fn new_page(&self, url: &str) -> Result<Page> {
        info!("Opening new tab: {}", url);
        self.0
            .new_page(url)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))
    }

    /// Get all pages.
    pub async fn pages(&self) -> Result<Vec<Page>> {
        self.0.pages().await.map_err(|e| Error::Cdp(e.to_string()))
    }
}

/// iframe context.
#[derive(Clone)]
pub struct IframeContext {
    /// Frame ID.
    pub frame_id: String,
    /// Frame URL.
    pub url: Option<String>,
    /// Frame content offset relative to the main viewport.
    pub offset_x: f64,
    /// Frame content offset relative to the main viewport.
    pub offset_y: f64,
}

/// Browser engine.
///
/// Core browser control interface, managing complete browser lifecycle and operations.
///
/// ## State Management
///
/// Internally maintains `active_page` and `tabs`, users don't need to manually track tab_id:
/// - `navigate()` automatically creates a page and sets it as active
/// - All operations default to the active page
/// - Supports multi-tab management
/// - Supports iframe context switching
///
/// ## Example
///
/// ```rust,no_run
/// use agent_browser_core::{ActionKind, BrowserEngine, BrowserConfig};
///
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// let engine = BrowserEngine::new(BrowserConfig::headed());
/// engine.navigate("https://example.com").await?;
/// let snapshot = engine.snapshot().await?;
/// engine
///     .act_with_snapshot(&snapshot.snapshot_id, "ax1", ActionKind::Click)
///     .await?;
/// engine.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct BrowserEngine {
    /// Browser instance.
    browser: Mutex<Option<Arc<Browser>>>,
    /// Tracks whether the CDP event loop is still alive.
    browser_alive: Arc<AtomicBool>,
    /// Serializes browser startup so concurrent first requests launch once.
    launch_lock: Mutex<()>,
    /// Combined tab state (single lock to prevent deadlocks).
    tab_state: Mutex<TabState>,
    /// Configuration.
    config: BrowserConfig,
    /// Combined iframe state (single lock to prevent deadlocks).
    iframe_state: Mutex<IframeState>,
    /// ref_id -> frame_id mapping (updated on each snapshot).
    iframe_mapping: Mutex<HashMap<String, String>>,
    /// Download directory.
    download_dir: Mutex<Option<PathBuf>>,
    /// Download event broadcaster.
    download_events: broadcast::Sender<DownloadResult>,
    /// Runtime lifecycle event broadcaster.
    event_tx: broadcast::Sender<BrowserEvent>,
    /// Network request log (for monitoring).
    network_requests: Arc<Mutex<Vec<crate::types::NetworkRequest>>>,
    /// Console message log (for monitoring).
    console_messages: Arc<Mutex<Vec<crate::types::ConsoleMessage>>>,
    network_monitoring_enabled: AtomicBool,
    console_monitoring_enabled: AtomicBool,
    network_monitor_pages: Mutex<HashSet<String>>,
    console_monitor_pages: Mutex<HashSet<String>>,
    /// Pages that already have the network policy interceptor installed.
    policy_pages: Mutex<HashSet<String>>,
    /// DNS policy decisions cached by hostname.
    network_policy_cache: Arc<Mutex<HashMap<String, (bool, Instant)>>>,
    /// Runtime URL patterns blocked by the agent.
    blocked_url_patterns: Arc<Mutex<Vec<String>>>,
    /// Latest page observation used to validate element references.
    snapshot_state: Mutex<Option<SnapshotState>>,
    /// Delta produced by the latest observation.
    last_snapshot_diff: Mutex<Option<SnapshotDiff>>,
}

/// Combined tab state to prevent deadlocks from acquiring multiple locks.
struct TabState {
    /// Active page.
    active_page: Option<Page>,
    /// Tab mapping (tab_id -> Page).
    tabs: HashMap<String, Page>,
    /// Active tab_id.
    active_tab_id: Option<String>,
}

/// Combined iframe state to prevent deadlocks from acquiring multiple locks.
#[derive(Clone)]
struct IframeState {
    /// iframe context stack (supports nested iframes).
    iframe_stack: Vec<IframeContext>,
    /// Currently active frame_id (None means main frame).
    active_frame_id: Option<String>,
    /// Currently active execution context ID.
    active_context_id: Option<i64>,
}

#[derive(Clone)]
struct SnapshotState {
    snapshot: PageSnapshot,
    page_id: String,
    frame_id: Option<String>,
}

struct DownloadListeners {
    begin: chromiumoxide::listeners::EventStream<
        chromiumoxide::cdp::browser_protocol::browser::EventDownloadWillBegin,
    >,
    progress: chromiumoxide::listeners::EventStream<
        chromiumoxide::cdp::browser_protocol::browser::EventDownloadProgress,
    >,
}

enum DownloadEvent {
    Begin(Arc<chromiumoxide::cdp::browser_protocol::browser::EventDownloadWillBegin>),
    Progress(Arc<chromiumoxide::cdp::browser_protocol::browser::EventDownloadProgress>),
}

impl BrowserEngine {
    /// Create a new browser engine (not launched).
    pub fn new(config: BrowserConfig) -> Self {
        let (download_events, _) = broadcast::channel(16);
        let (event_tx, _) = broadcast::channel(256);
        Self {
            browser: Mutex::new(None),
            browser_alive: Arc::new(AtomicBool::new(false)),
            launch_lock: Mutex::new(()),
            tab_state: Mutex::new(TabState {
                active_page: None,
                tabs: HashMap::new(),
                active_tab_id: None,
            }),
            config,
            iframe_state: Mutex::new(IframeState {
                iframe_stack: Vec::new(),
                active_frame_id: None,
                active_context_id: None,
            }),
            iframe_mapping: Mutex::new(HashMap::new()),
            download_dir: Mutex::new(None),
            download_events,
            event_tx,
            network_requests: Arc::new(Mutex::new(Vec::new())),
            console_messages: Arc::new(Mutex::new(Vec::new())),
            network_monitoring_enabled: AtomicBool::new(false),
            console_monitoring_enabled: AtomicBool::new(false),
            network_monitor_pages: Mutex::new(HashSet::new()),
            console_monitor_pages: Mutex::new(HashSet::new()),
            policy_pages: Mutex::new(HashSet::new()),
            network_policy_cache: Arc::new(Mutex::new(HashMap::new())),
            blocked_url_patterns: Arc::new(Mutex::new(Vec::new())),
            snapshot_state: Mutex::new(None),
            last_snapshot_diff: Mutex::new(None),
        }
    }

    /// Launch the browser.
    pub async fn launch(&self) -> Result<BrowserHandle> {
        let _launch_guard = self.launch_lock.lock().await;
        if self.browser_alive.load(Ordering::SeqCst)
            && let Some(browser) = self.browser.lock().await.as_ref().cloned()
        {
            return Ok(BrowserHandle(browser));
        }
        if self.browser.lock().await.is_some() {
            warn!("Discarding stale browser handle after CDP event loop ended");
            *self.browser.lock().await = None;
            let mut state = self.tab_state.lock().await;
            state.active_page = None;
            state.tabs.clear();
            state.active_tab_id = None;
            drop(state);
            self.reset_frame_state().await;
            self.policy_pages.lock().await.clear();
            self.network_monitor_pages.lock().await.clear();
            self.console_monitor_pages.lock().await.clear();
        }

        let headless_str = match self.config.headless {
            HeadlessMode::None => "headed",
            HeadlessMode::Old => "headless(old)",
            HeadlessMode::New => "headless(new)",
        };
        info!(
            "Launching browser (mode={}, stealth={})",
            headless_str, self.config.stealth
        );

        let mut builder = ChromeConfig::builder().request_timeout(Duration::from_millis(
            self.config
                .navigation_timeout_ms
                .max(self.config.action_timeout_ms),
        ));

        // 根据无头模式设置
        match self.config.headless {
            HeadlessMode::None => {
                builder = builder.with_head();
            }
            HeadlessMode::Old => {
                // 旧版无头模式（默认）
            }
            HeadlessMode::New => {
                // 新版无头模式：通过 arg 添加
                builder = builder.arg("--headless=new");
            }
        }

        // 添加反检测参数
        if self.config.stealth {
            builder = builder
                .arg("--disable-blink-features=AutomationControlled")
                .arg("--disable-infobars")
                .arg("--disable-dev-shm-usage")
                .arg("--disable-gpu")
                .arg("--window-size=1920,1080");
        }
        if self.config.no_sandbox {
            builder = builder.arg("--no-sandbox");
        }

        if let Some(ref dir) = self.config.profile_dir {
            info!("Profile directory: {:?}", dir);
            builder = builder.user_data_dir(dir);
        }

        if let Some(ref path) = self.config.browser_path {
            debug!("Browser executable: {}", path.display());
            builder = builder.chrome_executable(path);
        }

        // 添加用户自定义参数
        for arg in &self.config.extra_args {
            builder = builder.arg(arg.as_str());
        }

        let cfg = builder
            .build()
            .map_err(|e| Error::LaunchFailed(e.to_string()))?;

        let (browser, mut handler) = Browser::launch(cfg)
            .await
            .map_err(|e| Error::LaunchFailed(e.to_string()))?;

        // 后台驱动 CDP 事件循环
        let browser_alive = self.browser_alive.clone();
        let event_tx = self.event_tx.clone();
        browser_alive.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            use futures::StreamExt;
            while let Some(ev) = handler.next().await {
                debug!("Browser event: {:?}", ev);
            }
            if browser_alive.swap(false, Ordering::SeqCst) {
                let _ = event_tx.send(BrowserEvent::BrowserCrashed);
            }
            warn!("Browser handler stream ended");
        });

        let arc = Arc::new(browser);
        *self.browser.lock().await = Some(arc.clone());

        info!("Browser launched");
        Ok(BrowserHandle(arc))
    }

    /// Subscribe to browser lifecycle events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<BrowserEvent> {
        self.event_tx.subscribe()
    }

    /// 确保浏览器已启动
    async fn ensure_launched(&self) -> Result<BrowserHandle> {
        self.launch().await
    }

    /// 获取或创建活动页面
    async fn get_or_create_page(&self) -> Result<Page> {
        // 第一步：检查现有活动页面（持锁时间尽可能短）
        {
            let state = self.tab_state.lock().await;
            if let Some(ref page) = state.active_page {
                // 先克隆页面引用，释放锁后再检查有效性
                let page_clone = page.clone();
                drop(state);

                // 不持有锁的情况下检查页面有效性
                if page_clone.url().await.is_ok() {
                    self.ensure_network_policy(&page_clone).await?;
                    return Ok(page_clone);
                }
                // 页面无效，继续创建新页面
            }
            // 没有活动页面，继续创建新页面
        }

        // 第二步：创建新页面（锁已释放）
        let handle = self.ensure_launched().await?;
        let page = handle
            .new_page("about:blank")
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        // 在页面加载前注入 stealth 脚本
        if self.config.stealth {
            self.register_stealth_scripts(&page).await?;
        }
        self.ensure_network_policy(&page).await?;

        // 第三步：更新状态（重新获取锁）
        {
            let mut state = self.tab_state.lock().await;
            state.active_page = Some(page.clone());
        }

        Ok(page)
    }

    async fn ensure_network_policy(&self, page: &Page) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::fetch::{
            ContinueRequestParams, EnableParams, EventRequestPaused, FailRequestParams,
        };
        use chromiumoxide::cdp::browser_protocol::network::ErrorReason;
        use futures::StreamExt;

        let page_id = page.target_id().as_ref().to_string();
        {
            let mut installed = self.policy_pages.lock().await;
            if !installed.insert(page_id) {
                return Ok(());
            }
        }

        let mut events = page
            .event_listener::<EventRequestPaused>()
            .await
            .map_err(|error| Error::Cdp(error.to_string()))?;
        page.execute(EnableParams {
            patterns: None,
            handle_auth_requests: Some(false),
        })
        .await
        .map_err(|error| Error::Cdp(error.to_string()))?;

        let page = page.clone();
        let config = self.config.clone();
        let cache = self.network_policy_cache.clone();
        let blocked_url_patterns = self.blocked_url_patterns.clone();
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let request_id = event.request_id.clone();
                let runtime_blocked = blocked_url_patterns
                    .lock()
                    .await
                    .iter()
                    .any(|pattern| wildcard_match(pattern, &event.request.url));
                let decision = if runtime_blocked {
                    Err(Error::NetworkAccessDenied(format!(
                        "URL matches a runtime block rule: {}",
                        event.request.url
                    )))
                } else {
                    validate_url_policy(&config, &cache, &event.request.url).await
                };
                match decision {
                    Ok(_) => {
                        if let Err(error) =
                            page.execute(ContinueRequestParams::new(request_id)).await
                        {
                            warn!("Failed to continue policy-approved request: {error}");
                        }
                    }
                    Err(error) => {
                        warn!("Blocked browser request to {}: {error}", event.request.url);
                        if let Err(fail_error) = page
                            .execute(FailRequestParams::new(
                                request_id,
                                ErrorReason::BlockedByClient,
                            ))
                            .await
                        {
                            warn!("Failed to block policy-rejected request: {fail_error}");
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// 获取当前活动页面
    pub async fn active_page(&self) -> Result<Page> {
        let state = self.tab_state.lock().await;
        state.active_page.clone().ok_or(Error::NoActivePage)
    }

    async fn evaluate_in_active_context(&self, script: &str) -> Result<serde_json::Value> {
        use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};

        let page = self.active_page().await?;
        let context_id = self.iframe_state.lock().await.active_context_id;
        let result = if let Some(context_id) = context_id {
            let params = EvaluateParams::builder()
                .expression(script)
                .context_id(ExecutionContextId::new(context_id))
                .await_promise(true)
                .return_by_value(true)
                .build()
                .map_err(Error::InvalidParameter)?;
            page.evaluate_expression(params).await
        } else {
            page.evaluate(script).await
        }
        .map_err(|e| Error::JavaScript(e.to_string()))?;

        result
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))
    }

    async fn wait_for_actionable_selector(
        &self,
        selector: &str,
        require_editable: bool,
        timeout_ms: u64,
    ) -> Result<()> {
        let page = self.active_page().await?;
        let context_id = self.iframe_state.lock().await.active_context_id;
        crate::actions::wait_for_actionable(
            &page,
            context_id,
            selector,
            require_editable,
            timeout_ms,
        )
        .await
    }

    async fn reset_frame_state(&self) {
        let mut iframe_state = self.iframe_state.lock().await;
        iframe_state.iframe_stack.clear();
        iframe_state.active_frame_id = None;
        iframe_state.active_context_id = None;
        drop(iframe_state);
        self.iframe_mapping.lock().await.clear();
        *self.snapshot_state.lock().await = None;
        *self.last_snapshot_diff.lock().await = None;
    }

    async fn record_snapshot(&self, snapshot: &PageSnapshot) -> Result<()> {
        let page = self.active_page().await?;
        let frame_id = self.iframe_state.lock().await.active_frame_id.clone();
        let mut state = self.snapshot_state.lock().await;
        *self.last_snapshot_diff.lock().await = state
            .as_ref()
            .map(|previous| snapshot::diff_snapshots(&previous.snapshot, snapshot));
        *state = Some(SnapshotState {
            snapshot: snapshot.clone(),
            page_id: page.target_id().as_ref().to_string(),
            frame_id,
        });
        Ok(())
    }

    async fn validate_snapshot_ref(
        &self,
        snapshot_id: &str,
        ref_id: Option<&str>,
    ) -> Result<SnapshotState> {
        let state =
            self.snapshot_state
                .lock()
                .await
                .clone()
                .ok_or_else(|| Error::StaleSnapshot {
                    expected: snapshot_id.to_string(),
                    current: "none".to_string(),
                })?;
        if state.snapshot.snapshot_id != snapshot_id {
            return Err(Error::StaleSnapshot {
                expected: snapshot_id.to_string(),
                current: state.snapshot.snapshot_id,
            });
        }

        let page = self.active_page().await?;
        let current_page_id = page.target_id().as_ref().to_string();
        let current_frame_id = self.iframe_state.lock().await.active_frame_id.clone();
        if state.page_id != current_page_id || state.frame_id != current_frame_id {
            return Err(Error::StaleSnapshot {
                expected: snapshot_id.to_string(),
                current: "page-context-changed".to_string(),
            });
        }
        if let Some(ref_id) = ref_id
            && snapshot::find_node_by_ref(&state.snapshot.nodes, ref_id).is_none()
        {
            return Err(Error::ElementNotFound(ref_id.to_string()));
        }
        Ok(state)
    }

    /// Return the latest snapshot identifier for the active page context.
    pub async fn current_snapshot_id(&self) -> Option<String> {
        self.snapshot_state
            .lock()
            .await
            .as_ref()
            .map(|state| state.snapshot.snapshot_id.clone())
    }

    /// Capture and compact an accessibility snapshot for an agent context window.
    pub async fn snapshot_with_options(&self, options: SnapshotOptions) -> Result<PageSnapshot> {
        let snapshot = self.snapshot().await?;
        Ok(snapshot::compact_snapshot(&snapshot, &options))
    }

    /// Search the latest snapshot, capturing one first when needed.
    pub async fn search_snapshot(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<SnapshotSearchResult> {
        if query.trim().is_empty() {
            return Err(Error::InvalidParameter(
                "Snapshot search query cannot be empty".to_string(),
            ));
        }
        let cached = self
            .snapshot_state
            .lock()
            .await
            .as_ref()
            .map(|state| state.snapshot.clone());
        let snapshot = match cached {
            Some(snapshot) => snapshot,
            None => self.snapshot().await?,
        };
        Ok(snapshot::search_snapshot(&snapshot, query, max_results))
    }

    /// Return the delta between the latest two observations.
    pub async fn latest_snapshot_diff(&self) -> Option<SnapshotDiff> {
        self.last_snapshot_diff.lock().await.clone()
    }

    /// 导航到 URL
    ///
    /// 如果浏览器未启动会自动启动。
    /// 导航成功后更新活动页面。
    pub async fn navigate(&self, url: &str) -> Result<crate::types::NavigateResult> {
        self.navigate_with_options(url, NavigationWaitUntil::default())
            .await
    }

    /// 导航到 URL，带等待策略选项
    ///
    /// # 参数
    ///
    /// - `url`: 目标 URL（必须以 http:// 或 https:// 开头）
    /// - `wait_until`: 等待策略（Load / DomContentLoaded / NetworkIdle / None）
    pub async fn navigate_with_options(
        &self,
        url: &str,
        wait_until: NavigationWaitUntil,
    ) -> Result<crate::types::NavigateResult> {
        use chromiumoxide::cdp::browser_protocol::page::EventLifecycleEvent;
        use futures::StreamExt;

        let parsed_url = validate_url_policy(&self.config, &self.network_policy_cache, url).await?;
        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Err(Error::InvalidParameter(
                "Navigation URL must use http:// or https://".to_string(),
            ));
        }

        info!("Navigating to: {} (wait_until: {:?})", url, wait_until);

        let page = self.get_or_create_page().await?;

        use chromiumoxide::cdp::browser_protocol::page::NavigateParams;

        self.reset_frame_state().await;
        let timeout = Duration::from_millis(self.config.navigation_timeout_ms);
        let mut lifecycle_events = if wait_until == NavigationWaitUntil::None {
            None
        } else {
            Some(
                page.event_listener::<EventLifecycleEvent>()
                    .await
                    .map_err(|e| Error::Cdp(e.to_string()))?,
            )
        };

        let navigation = tokio::time::timeout(timeout, page.execute(NavigateParams::new(url)))
            .await
            .map_err(|_| Error::Timeout("Navigation command timed out".to_string()))?
            .map_err(|e| Error::Cdp(e.to_string()))?
            .result;

        if let Some(error_text) = navigation.error_text {
            return Err(Error::Cdp(error_text));
        }

        let expected_event = match wait_until {
            NavigationWaitUntil::Load | NavigationWaitUntil::NetworkIdle => Some("load"),
            NavigationWaitUntil::DomContentLoaded => Some("DOMContentLoaded"),
            NavigationWaitUntil::None => None,
        };

        if let (Some(expected_event), Some(stream)) = (expected_event, lifecycle_events.as_mut()) {
            let frame_id = navigation.frame_id.clone();
            let loader_id = navigation.loader_id.clone();
            tokio::time::timeout(timeout, async {
                while let Some(event) = stream.next().await {
                    if event.frame_id != frame_id || event.name != expected_event {
                        continue;
                    }
                    if loader_id
                        .as_ref()
                        .is_some_and(|expected| event.loader_id != *expected)
                    {
                        continue;
                    }
                    return Ok::<(), Error>(());
                }
                Err(Error::Cdp(
                    "Navigation lifecycle event stream closed".to_string(),
                ))
            })
            .await
            .map_err(|_| {
                Error::Timeout(format!("Navigation timeout waiting for {expected_event}"))
            })??;
        }

        if wait_until == NavigationWaitUntil::NetworkIdle {
            self.wait_for_network_idle(500, self.config.navigation_timeout_ms)
                .await?;
        }

        let final_url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| url.to_string());
        validate_url_policy(&self.config, &self.network_policy_cache, &final_url).await?;

        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        info!("Navigated to: {} (title: {})", final_url, title);
        let _ = self.event_tx.send(BrowserEvent::Navigated {
            url: final_url.clone(),
            title: title.clone(),
        });

        // 注册到 tabs（如果尚未注册）
        {
            let mut state = self.tab_state.lock().await;
            let active_id = state.active_tab_id.clone();
            if let Some(ref existing_id) = active_id {
                // 确保活动页在 tabs 中
                if !state.tabs.contains_key(existing_id) {
                    state.tabs.insert(existing_id.clone(), page.clone());
                }
            } else {
                let tab_id = format!("tab-{}", uuid::Uuid::new_v4());
                state.tabs.insert(tab_id.clone(), page.clone());
                state.active_tab_id = Some(tab_id);
            }
        }

        Ok(crate::types::NavigateResult {
            url: url.to_string(),
            title: title.clone(),
            final_url,
        })
    }

    /// 注册反检测脚本（在每个新文档加载前自动执行）
    ///
    /// 使用 CDP Page.addScriptToEvaluateOnNewDocument 在页面 JS 执行之前注入脚本，
    /// 确保反检测在页面检测脚本运行之前生效。
    async fn register_stealth_scripts(&self, page: &Page) -> Result<()> {
        let stealth_js = r#"
            // 隐藏 webdriver 标志
            Object.defineProperty(navigator, 'webdriver', {
                get: () => undefined
            });

            // 模拟真实的 plugins
            Object.defineProperty(navigator, 'plugins', {
                get: () => {
                    const plugins = [
                        { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
                        { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
                        { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' }
                    ];
                    plugins.item = (index) => plugins[index] || null;
                    plugins.namedItem = (name) => plugins.find(p => p.name === name) || null;
                    plugins.refresh = () => {};
                    return plugins;
                }
            });

            // 模拟真实的 languages
            Object.defineProperty(navigator, 'languages', {
                get: () => ['zh-CN', 'zh', 'en']
            });

            // 隐藏自动化相关的 Chrome 属性
            if (window.chrome) {
                window.chrome.runtime = {};
            }

            // 覆盖 permissions API
            const originalQuery = window.navigator.permissions.query;
            window.navigator.permissions.query = (parameters) => (
                parameters.name === 'notifications' ?
                    Promise.resolve({ state: Notification.permission }) :
                    originalQuery(parameters)
            );

            // 隐藏自动化特征
            delete window.__webdriver_evaluate;
            delete window.__selenium_evaluate;
            delete window.__webdriver_script_function;
            delete window.__webdriver_script_func;
            delete window.__webdriver_script_fn;
            delete window.__fxdriver_evaluate;
            delete window.__driver_unwrapped;
            delete window.__webdriver_unwrapped;
            delete window.__driver_evaluate;
            delete window.__selenium_unwrapped;
            delete window.__fxdriver_unwrapped;

            console.log('[Stealth] Anti-detection scripts injected');
        "#;

        page.evaluate_on_new_document(stealth_js)
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        debug!("Stealth scripts registered with addScriptToEvaluateOnNewDocument");
        Ok(())
    }

    /// 通过 CSS 选择器点击元素
    ///
    /// 直接使用 CSS 选择器定位并点击元素，无需先获取快照。
    /// 适用于动态内容或需要精确控制的场景。
    ///
    /// # 参数
    ///
    /// - `selector`: CSS 选择器，如 "#submit-btn", ".login-form button", "[data-id='123']"
    /// - `timeout_ms`: 等待元素出现的超时时间（毫秒）
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use agent_browser_core::{BrowserEngine, BrowserConfig};
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let engine = BrowserEngine::new(BrowserConfig::default());
    /// engine.click_selector("#submit-btn", Some(5000)).await?;
    /// engine.click_selector("a[href='/logout']", Some(3000)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn click_selector(
        &self,
        selector: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        info!("Clicking element by selector: {}", selector);

        // 等待元素出现
        self.wait_for_selector(selector, timeout).await?;
        self.wait_for_actionable_selector(selector, false, timeout)
            .await?;

        let page = self.active_page().await?;
        let (context_id, offset) = {
            let state = self.iframe_state.lock().await;
            let offset = state
                .iframe_stack
                .last()
                .map(|frame| (frame.offset_x, frame.offset_y))
                .unwrap_or((0.0, 0.0));
            (state.active_context_id, offset)
        };
        crate::actions::dispatch_pointer_action(
            &page,
            context_id,
            selector,
            &ActionKind::Click,
            offset,
        )
        .await
    }

    /// 通过 CSS 选择器输入文本
    ///
    /// 在匹配选择器的输入框中输入文本。
    ///
    /// # 参数
    ///
    /// - `selector`: CSS 选择器
    /// - `text`: 要输入的文本
    /// - `clear_first`: 是否先清空输入框
    /// - `timeout_ms`: 等待超时
    pub async fn type_selector(
        &self,
        selector: &str,
        text: &str,
        clear_first: bool,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        info!("Typing in element by selector: {}", selector);

        // 等待元素出现
        self.wait_for_selector(selector, timeout).await?;
        self.wait_for_actionable_selector(selector, true, timeout)
            .await?;

        let focus_script = format!(
            r#"(() => {{
                const el = document.querySelector({selector:?});
                if (!el) return false;
                el.focus();
                if ({clear_first}) {{
                    if (typeof el.select === 'function') el.select();
                    else {{
                        const range = document.createRange();
                        range.selectNodeContents(el);
                        const selection = getSelection();
                        selection.removeAllRanges();
                        selection.addRange(range);
                    }}
                }} else if (typeof el.setSelectionRange === 'function') {{
                    const end = String(el.value || '').length;
                    el.setSelectionRange(end, end);
                }}
                return true;
            }})()"#
        );
        let focused: bool =
            serde_json::from_value(self.evaluate_in_active_context(&focus_script).await?)?;
        if !focused {
            return Err(Error::ElementNotFound(selector.to_string()));
        }

        use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
        let page = self.active_page().await?;
        page.execute(InsertTextParams::new(text))
            .await
            .map_err(|error| Error::Cdp(error.to_string()))?;

        Ok(ActionResult {
            success: true,
            message: format!("Typed {} characters", text.chars().count()),
        })
    }

    /// 获取元素的文本内容
    ///
    /// 通过 CSS 选择器获取元素的文本内容。
    pub async fn get_text(&self, selector: &str, timeout_ms: Option<u64>) -> Result<String> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(selector, timeout).await?;

        let script = format!(
            r#"document.querySelector({sel:?})?.textContent?.trim() || ''"#,
            sel = selector
        );

        let result: String =
            serde_json::from_value(self.evaluate_in_active_context(script.as_str()).await?)?;

        Ok(result)
    }

    /// 获取元素的属性值
    ///
    /// 通过 CSS 选择器获取元素的指定属性。
    pub async fn get_attribute(
        &self,
        selector: &str,
        attribute: &str,
        timeout_ms: Option<u64>,
    ) -> Result<Option<String>> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(selector, timeout).await?;

        let script = format!(
            r#"document.querySelector({sel:?})?.getAttribute({attr:?})"#,
            sel = selector,
            attr = attribute
        );

        let result: Option<String> =
            serde_json::from_value(self.evaluate_in_active_context(script.as_str()).await?)?;

        Ok(result)
    }

    /// 检查元素是否存在
    pub async fn element_exists(&self, selector: &str) -> Result<bool> {
        let script = format!(
            r#"document.querySelector({sel:?}) !== null"#,
            sel = selector
        );

        let result: bool =
            serde_json::from_value(self.evaluate_in_active_context(script.as_str()).await?)?;

        Ok(result)
    }

    /// 展开子菜单/折叠面板
    ///
    /// 用于处理 Vue/Element UI 等框架的 el-submenu、el-collapse 等组件。
    /// 先尝试点击展开按钮，等待子菜单出现。
    ///
    /// # 参数
    ///
    /// - `menu_selector`: 菜单项选择器
    /// - `submenu_selector`: 子菜单项选择器
    /// - `timeout_ms`: 超时时间
    pub async fn expand_and_click_submenu(
        &self,
        menu_selector: &str,
        submenu_selector: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        info!(
            "Expanding menu '{}' and clicking submenu '{}'",
            menu_selector, submenu_selector
        );

        // 检查子菜单是否已可见
        let submenu_visible = self.element_exists(submenu_selector).await.unwrap_or(false);

        if !submenu_visible {
            // 点击菜单项展开
            self.click_selector(menu_selector, Some(timeout)).await?;

            // 等待子菜单出现
            self.wait_for_selector(submenu_selector, timeout).await?;
        }

        // 点击子菜单
        self.click_selector(submenu_selector, Some(timeout)).await
    }

    /// 选择下拉框选项（支持 select 和自定义下拉组件）
    ///
    /// # 参数
    ///
    /// - `select_selector`: 下拉框选择器
    /// - `value`: 要选择的值（value 或 text）
    /// - `by_text`: 是否按文本匹配（默认按 value 匹配）
    pub async fn select_option(
        &self,
        select_selector: &str,
        value: &str,
        by_text: bool,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(select_selector, timeout).await?;
        self.wait_for_actionable_selector(select_selector, false, timeout)
            .await?;

        // 尝试标准 select 元素
        let script = if by_text {
            format!(
                r#"(function() {{
                    const select = document.querySelector({sel:?});
                    if (select && select.tagName === 'SELECT') {{
                        for (let opt of select.options) {{
                            if (opt.text === {val:?}) {{
                                select.value = opt.value;
                                select.dispatchEvent(new Event('change', {{ bubbles: true }}));
                                return {{ success: true, selected: opt.text }};
                            }}
                        }}
                    }}
                    return {{ success: false, error: 'Option not found' }};
                }})()"#,
                sel = select_selector,
                val = value
            )
        } else {
            format!(
                r#"(function() {{
                    const select = document.querySelector({sel:?});
                    if (select && select.tagName === 'SELECT') {{
                        select.value = {val:?};
                        select.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        return {{ success: true, selected: select.value }};
                    }}
                    return {{ success: false, error: 'Not a select element' }};
                }})()"#,
                sel = select_selector,
                val = value
            )
        };

        let result = self.evaluate_in_active_context(script.as_str()).await?;

        let success = result["success"].as_bool().unwrap_or(false);
        if success {
            let selected = result["selected"].as_str().unwrap_or("");
            Ok(ActionResult {
                success: true,
                message: format!("Selected: {}", selected),
            })
        } else {
            // 尝试点击自定义下拉组件
            self.click_selector(select_selector, Some(timeout)).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;

            // 查找并点击选项（使用 JavaScript 避免 CSS 选择器注入风险）
            let click_script = format!(
                r#"(function() {{
                    const value = {val:?};
                    // 按属性查找并点击
                    const byAttr = document.querySelector("[data-value='" + value + "'], [title='" + value + "'], [value='" + value + "']");
                    if (byAttr) {{
                        byAttr.click();
                        return {{ success: true, clicked: byAttr.outerHTML }};
                    }}
                    // 按文本内容查找并点击（仅在 by_text 模式下）
                    if ({by_text}) {{
                        const clickable = document.querySelectorAll('button, a, [role="button"], [role="option"], li, div[clickable], span[clickable]');
                        for (const el of clickable) {{
                            if (el.textContent.trim() === value || el.innerText.trim() === value) {{
                                el.click();
                                return {{ success: true, clicked: el.outerHTML }};
                            }}
                        }}
                    }}
                    return {{ success: false, error: 'Option not found' }};
                }})()"#,
                val = value,
                by_text = by_text
            );

            let click_result = self
                .evaluate_in_active_context(click_script.as_str())
                .await?;

            let clicked = click_result["success"].as_bool().unwrap_or(false);
            if clicked {
                Ok(ActionResult {
                    success: true,
                    message: format!("Selected option: {}", value),
                })
            } else {
                Err(Error::ElementNotFound(format!(
                    "Option '{}' not found",
                    value
                )))
            }
        }
    }

    /// 模拟真实用户鼠标悬停
    ///
    /// 触发 mouseover、mouseenter 事件，用于显示隐藏的菜单等。
    pub async fn hover_selector(
        &self,
        selector: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(selector, timeout).await?;
        self.wait_for_actionable_selector(selector, false, timeout)
            .await?;
        let page = self.active_page().await?;
        let (context_id, offset) = {
            let state = self.iframe_state.lock().await;
            let offset = state
                .iframe_stack
                .last()
                .map(|frame| (frame.offset_x, frame.offset_y))
                .unwrap_or((0.0, 0.0));
            (state.active_context_id, offset)
        };
        crate::actions::dispatch_pointer_action(
            &page,
            context_id,
            selector,
            &ActionKind::Hover,
            offset,
        )
        .await
    }

    /// 获取页面快照
    ///
    /// 返回 Accessibility Tree，包含所有可交互元素的 ref_id、role、name。
    /// 如果当前在 iframe 上下文中，将获取该 iframe 内的元素。
    /// 同时更新 iframe 映射表。
    pub async fn snapshot(&self) -> Result<PageSnapshot> {
        let active_frame = self.iframe_state.lock().await.active_frame_id.clone();

        // 检查是否在 iframe 上下文中
        if let Some(frame_id) = active_frame {
            info!("Taking snapshot in iframe context: {}", frame_id);
            let snapshot = self.snapshot_in_frame().await?;
            self.record_snapshot(&snapshot).await?;
            return Ok(snapshot);
        }

        // 主 frame 上下文
        let page = self.active_page().await?;
        let snapshot = snapshot::generate_snapshot(&page).await?;

        // 更新 iframe 映射
        let mut mapping = self.iframe_mapping.lock().await;
        mapping.clear();
        for m in &snapshot.iframe_mappings {
            mapping.insert(m.ref_id.clone(), m.frame_id.clone());
        }
        info!("Updated {} iframe mappings", mapping.len());
        drop(mapping);
        self.record_snapshot(&snapshot).await?;

        Ok(snapshot)
    }

    /// 执行元素操作
    ///
    /// # 参数
    ///
    /// - `ref_id`: 元素引用 ID（如 "ax1", "e5"）
    /// - `action`: 动作类型
    pub async fn act(&self, ref_id: &str, action: ActionKind) -> Result<ActionResult> {
        let page = self.active_page().await?;
        let (context_id, frame_offset) = {
            let state = self.iframe_state.lock().await;
            let offset = state
                .iframe_stack
                .last()
                .map(|frame| (frame.offset_x, frame.offset_y))
                .unwrap_or((0.0, 0.0));
            (state.active_context_id, offset)
        };
        let action_name = format!("{:?}", action);
        let result = if let Some(context_id) = context_id {
            crate::actions::dispatch_action_in_context(
                &page,
                context_id,
                ref_id,
                action,
                crate::actions::ContextActionOptions {
                    snapshot_url: None,
                    snapshot_id: None,
                    timeout_ms: self.config.action_timeout_ms,
                    frame_offset,
                },
            )
            .await
        } else {
            crate::actions::dispatch_action(
                &page,
                ref_id,
                action,
                None,
                None,
                self.config.action_timeout_ms,
            )
            .await
        }?;
        let _ = self.event_tx.send(BrowserEvent::ActionCompleted {
            action: action_name,
            ref_id: ref_id.to_string(),
        });
        Ok(result)
    }

    /// Execute an action only if the element still belongs to the supplied snapshot.
    pub async fn act_with_snapshot(
        &self,
        snapshot_id: &str,
        ref_id: &str,
        action: ActionKind,
    ) -> Result<ActionResult> {
        let state = self
            .validate_snapshot_ref(
                snapshot_id,
                (!matches!(&action, ActionKind::Scroll { .. } | ActionKind::Wait { .. }))
                    .then_some(ref_id),
            )
            .await?;
        let page = self.active_page().await?;
        let (context_id, frame_offset) = {
            let state = self.iframe_state.lock().await;
            let offset = state
                .iframe_stack
                .last()
                .map(|frame| (frame.offset_x, frame.offset_y))
                .unwrap_or((0.0, 0.0));
            (state.active_context_id, offset)
        };
        let action_name = format!("{:?}", action);
        let result = if let Some(context_id) = context_id {
            crate::actions::dispatch_action_in_context(
                &page,
                context_id,
                ref_id,
                action,
                crate::actions::ContextActionOptions {
                    snapshot_url: Some(&state.snapshot.url),
                    snapshot_id: Some(snapshot_id),
                    timeout_ms: self.config.action_timeout_ms,
                    frame_offset,
                },
            )
            .await
        } else {
            crate::actions::dispatch_action(
                &page,
                ref_id,
                action,
                Some(&state.snapshot.url),
                Some(snapshot_id),
                self.config.action_timeout_ms,
            )
            .await
        }?;
        let _ = self.event_tx.send(BrowserEvent::ActionCompleted {
            action: action_name,
            ref_id: ref_id.to_string(),
        });
        Ok(result)
    }

    /// 点击元素
    pub async fn click(&self, ref_id: &str) -> Result<ActionResult> {
        self.act(ref_id, ActionKind::Click).await
    }

    /// 双击元素
    pub async fn double_click(&self, ref_id: &str) -> Result<ActionResult> {
        self.act(ref_id, ActionKind::DoubleClick).await
    }

    /// 右键点击元素
    pub async fn right_click(&self, ref_id: &str) -> Result<ActionResult> {
        self.act(ref_id, ActionKind::RightClick).await
    }

    /// 悬停在元素上
    pub async fn hover(&self, ref_id: &str) -> Result<ActionResult> {
        self.act(ref_id, ActionKind::Hover).await
    }

    /// 聚焦元素
    pub async fn focus(&self, ref_id: &str) -> Result<ActionResult> {
        self.act(ref_id, ActionKind::Focus).await
    }

    /// 输入文本
    pub async fn type_text(
        &self,
        ref_id: &str,
        text: &str,
        clear_first: bool,
    ) -> Result<ActionResult> {
        self.act(
            ref_id,
            ActionKind::Type {
                text: text.to_string(),
                clear_first: Some(clear_first),
            },
        )
        .await
    }

    /// 按键
    pub async fn press(&self, ref_id: &str, key: &str) -> Result<ActionResult> {
        self.act(
            ref_id,
            ActionKind::Press {
                key: key.to_string(),
            },
        )
        .await
    }

    /// 选择选项
    pub async fn select(&self, ref_id: &str, values: Vec<String>) -> Result<ActionResult> {
        self.act(ref_id, ActionKind::Select { values }).await
    }

    /// 拖拽元素
    pub async fn drag(&self, ref_id: &str, target_ref_id: &str) -> Result<ActionResult> {
        self.act(
            ref_id,
            ActionKind::Drag {
                target_ref_id: target_ref_id.to_string(),
            },
        )
        .await
    }

    /// 滚动页面
    pub async fn scroll(&self, direction: &str, amount: i32) -> Result<ActionResult> {
        self.act(
            "",
            ActionKind::Scroll {
                direction: Some(direction.to_string()),
                amount: Some(amount),
            },
        )
        .await
    }

    /// 截图
    pub async fn screenshot(&self) -> Result<crate::types::ScreenshotResult> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use chromiumoxide::cdp::browser_protocol::page::{
            CaptureScreenshotFormat, CaptureScreenshotParams,
        };

        let page = self.active_page().await?;

        let params = CaptureScreenshotParams {
            format: Some(CaptureScreenshotFormat::Png),
            ..Default::default()
        };

        let result = page
            .execute(params)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        // result.data is Binary (base64-encoded string wrapper)
        // Get the base64 string directly
        let data: String = result.data.clone().into();

        // 解码 base64 以获取图片尺寸
        let decoded = STANDARD
            .decode(&data)
            .map_err(|e| Error::Other(format!("Base64 decode error: {}", e)))?;

        // 解析图片尺寸（PNG 头）
        let (width, height) = parse_png_dimensions(&decoded).unwrap_or((0, 0));

        Ok(crate::types::ScreenshotResult {
            data,
            format: "png".to_string(),
            width,
            height,
        })
    }

    /// 执行 JavaScript
    pub async fn evaluate(&self, script: &str) -> Result<serde_json::Value> {
        self.evaluate_in_active_context(script).await
    }

    /// 等待指定时间
    pub async fn wait(&self, timeout_ms: u64) -> Result<()> {
        tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
        Ok(())
    }

    /// 获取当前 URL
    pub async fn current_url(&self) -> Result<String> {
        let page = self.active_page().await?;
        page.url()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?
            .ok_or(Error::NoActivePage)
    }

    /// 获取页面标题
    pub async fn title(&self) -> Result<String> {
        let page = self.active_page().await?;
        page.get_title()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?
            .ok_or(Error::NoActivePage)
    }

    /// 健康检查
    pub async fn health_check(&self) -> bool {
        if !self.browser_alive.load(Ordering::SeqCst) {
            return false;
        }
        let guard = self.browser.lock().await;
        if let Some(ref browser) = *guard {
            browser.pages().await.is_ok()
        } else {
            false
        }
    }

    /// Whether a browser instance has been launched.
    pub async fn is_launched(&self) -> bool {
        self.browser_alive.load(Ordering::SeqCst) && self.browser.lock().await.is_some()
    }

    /// 关闭浏览器
    pub async fn shutdown(&self) -> Result<()> {
        // Mark the shutdown as intentional before closing CDP so the handler task
        // does not emit a BrowserCrashed event.
        self.browser_alive.store(false, Ordering::SeqCst);
        let mut browser_guard = self.browser.lock().await;
        let mut state = self.tab_state.lock().await;

        // 清空活动页面
        state.active_page = None;
        state.tabs.clear();
        state.active_tab_id = None;

        if let Some(browser) = browser_guard.take() {
            // 关闭所有页面
            if let Ok(pages) = browser.pages().await {
                for page in pages {
                    let _ = page.close().await;
                }
            }

            // 尝试关闭浏览器
            match Arc::try_unwrap(browser) {
                Ok(mut b) => {
                    b.close().await.map_err(|e| Error::Cdp(e.to_string()))?;
                }
                Err(_) => {
                    warn!("Browser handle clones still exist");
                }
            }

            info!("Browser shutdown complete");
        }

        drop(state);
        drop(browser_guard);
        self.policy_pages.lock().await.clear();
        self.network_monitor_pages.lock().await.clear();
        self.console_monitor_pages.lock().await.clear();
        self.network_requests.lock().await.clear();
        self.console_messages.lock().await.clear();
        self.network_policy_cache.lock().await.clear();
        self.iframe_mapping.lock().await.clear();
        *self.snapshot_state.lock().await = None;
        *self.last_snapshot_diff.lock().await = None;
        *self.iframe_state.lock().await = IframeState {
            iframe_stack: Vec::new(),
            active_frame_id: None,
            active_context_id: None,
        };

        Ok(())
    }
}

/// 解析 PNG 图片尺寸
fn parse_png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 {
        return None;
    }

    // PNG 头: 8 bytes signature + 4 bytes length + 4 bytes type + 4 bytes width + 4 bytes height
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);

    Some((width, height))
}

fn collect_snapshot_nodes<'a>(
    nodes: &'a [crate::snapshot::SnapshotNode],
    output: &mut Vec<&'a crate::snapshot::SnapshotNode>,
) {
    for node in nodes {
        output.push(node);
        collect_snapshot_nodes(&node.children, output);
    }
}

// ---------------------------------------------------------------------------
// Cookie 管理
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 获取当前页面的所有 Cookie
    pub async fn get_cookies(&self) -> Result<Vec<CookieInfo>> {
        let page = self.active_page().await?;
        let cookies = page
            .get_cookies()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        Ok(cookies
            .into_iter()
            .map(|c| CookieInfo {
                name: c.name,
                value: c.value,
                domain: Some(c.domain),
                path: Some(c.path),
                secure: c.secure,
                http_only: c.http_only,
            })
            .collect())
    }

    /// 设置 Cookie
    pub async fn set_cookies(&self, cookies: Vec<SetCookieParam>) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::network::CookieParam;

        let page = self.active_page().await?;

        for c in cookies {
            page.set_cookie(CookieParam {
                name: c.name,
                value: c.value,
                domain: c.domain,
                path: c.path,
                secure: c.secure,
                http_only: c.http_only,
                expires: None,
                same_site: None,
                url: None,
                priority: None,
                same_party: None,
                source_scheme: None,
                source_port: None,
                partition_key: None,
            })
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 多标签页管理
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// Open a new active tab and navigate it to a URL.
    pub async fn new_tab(&self, url: &str) -> Result<TabInfo> {
        validate_url_policy(&self.config, &self.network_policy_cache, url).await?;
        let handle = self.ensure_launched().await?;
        let page = handle.new_page("about:blank").await?;
        if self.config.stealth {
            self.register_stealth_scripts(&page).await?;
        }
        self.ensure_network_policy(&page).await?;
        let tab_id = format!("tab-{}", uuid::Uuid::new_v4());
        {
            let mut state = self.tab_state.lock().await;
            state.tabs.insert(tab_id.clone(), page.clone());
            state.active_tab_id = Some(tab_id.clone());
            state.active_page = Some(page);
        }
        self.reset_frame_state().await;
        let navigation = self
            .navigate_with_options(url, NavigationWaitUntil::Load)
            .await?;
        Ok(TabInfo {
            tab_id,
            url: navigation.final_url,
            title: navigation.title,
            active: true,
        })
    }

    /// 列出所有标签页
    pub async fn list_tabs(&self) -> Result<Vec<TabInfo>> {
        // 第一步：克隆必要数据（持锁时间尽可能短）
        let (tabs_clone, active_tab_id): (Vec<(String, Page)>, Option<String>) = {
            let state = self.tab_state.lock().await;
            (
                state
                    .tabs
                    .iter()
                    .map(|(id, p)| (id.clone(), p.clone()))
                    .collect(),
                state.active_tab_id.clone(),
            )
        };
        // 锁已释放

        // 第二步：不持锁的情况下查询每个页面
        let mut result = Vec::new();
        for (tab_id, page) in tabs_clone {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            let title = page.get_title().await.ok().flatten().unwrap_or_default();
            let active = Some(tab_id.as_str()) == active_tab_id.as_deref();

            result.push(TabInfo {
                tab_id,
                url,
                title,
                active,
            });
        }

        Ok(result)
    }

    /// 激活标签页
    pub async fn activate_tab(&self, tab_id: &str) -> Result<()> {
        {
            let mut state = self.tab_state.lock().await;
            if !state.tabs.contains_key(tab_id) {
                return Err(Error::Other(format!("Tab not found: {tab_id}")));
            }
            state.active_tab_id = Some(tab_id.to_string());
            state.active_page = state.tabs.get(tab_id).cloned();
        }
        self.reset_frame_state().await;
        if self.network_monitoring_enabled.load(Ordering::SeqCst) {
            self.enable_network_monitoring().await?;
        }
        if self.console_monitoring_enabled.load(Ordering::SeqCst) {
            self.enable_console_monitoring().await?;
        }

        info!("Activated tab: {}", tab_id);
        let _ = self.event_tx.send(BrowserEvent::TabActivated {
            tab_id: tab_id.to_string(),
        });
        Ok(())
    }

    /// 关闭标签页
    pub async fn close_tab(&self, tab_id: &str) -> Result<()> {
        let (page, was_active) = {
            let state = self.tab_state.lock().await;
            let page = state
                .tabs
                .get(tab_id)
                .cloned()
                .ok_or_else(|| Error::Other(format!("Tab not found: {tab_id}")))?;
            (page, state.active_tab_id.as_deref() == Some(tab_id))
        };

        page.close().await.map_err(|e| Error::Cdp(e.to_string()))?;

        {
            let mut state = self.tab_state.lock().await;
            state.tabs.remove(tab_id);
            if was_active {
                state.active_tab_id = state.tabs.keys().next().cloned();
                state.active_page = state
                    .active_tab_id
                    .as_ref()
                    .and_then(|id| state.tabs.get(id).cloned());
            }
        }
        if was_active {
            self.reset_frame_state().await;
        }
        let _ = self.event_tx.send(BrowserEvent::TabClosed {
            tab_id: tab_id.to_string(),
        });

        info!("Closed tab: {}", tab_id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 高级截图
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 高级截图（支持全页面和指定元素）
    pub async fn screenshot_with_options(
        &self,
        options: ScreenshotOptions,
    ) -> Result<crate::types::ScreenshotResult> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use chromiumoxide::cdp::browser_protocol::page::Viewport;
        use chromiumoxide::page::ScreenshotParams;

        let page = self.active_page().await?;

        // 如果指定了选择器，截取该元素
        if let Some(ref selector) = options.selector {
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({sel:?});
                    if (!el) return null;
                    const r = el.getBoundingClientRect();
                    return {{x:r.x, y:r.y, width:r.width, height:r.height}};
                }})()"#,
                sel = selector
            );

            let bounds = self.evaluate_in_active_context(js.as_str()).await.ok();

            if let Some(b) = bounds {
                let x = b["x"].as_f64().unwrap_or(0.0);
                let y = b["y"].as_f64().unwrap_or(0.0);
                let w = b["width"].as_f64().unwrap_or(0.0);
                let h = b["height"].as_f64().unwrap_or(0.0);

                if w > 0.0 && h > 0.0 {
                    let params = ScreenshotParams::builder()
                        .clip(Viewport {
                            x,
                            y,
                            width: w,
                            height: h,
                            scale: 1.0,
                        })
                        .build();

                    let data = page
                        .screenshot(params)
                        .await
                        .map_err(|e| Error::Cdp(e.to_string()))?;

                    let (width, height) = parse_png_dimensions(&data).unwrap_or((0, 0));

                    return Ok(crate::types::ScreenshotResult {
                        data: STANDARD.encode(&data),
                        format: "png".to_string(),
                        width,
                        height,
                    });
                }
            }

            warn!(
                "Selector '{}' not found, falling back to viewport",
                selector
            );
        }

        // 全页面或视口截图
        let params = if options.full_page.unwrap_or(false) {
            ScreenshotParams::builder().full_page(true).build()
        } else {
            ScreenshotParams::builder().build()
        };

        let data = page
            .screenshot(params)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let (width, height) = parse_png_dimensions(&data).unwrap_or((0, 0));

        Ok(crate::types::ScreenshotResult {
            data: STANDARD.encode(&data),
            format: "png".to_string(),
            width,
            height,
        })
    }
}

// ---------------------------------------------------------------------------
// 等待功能
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 等待选择器出现
    pub async fn wait_for_selector(&self, selector: &str, timeout_ms: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        loop {
            let js = format!("!!document.querySelector({:?})", selector);
            let found: bool = serde_json::from_value(self.evaluate_in_active_context(&js).await?)?;

            if found {
                return Ok(());
            }

            if Instant::now() >= deadline {
                return Err(Error::Timeout(format!("Selector not found: {}", selector)));
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 等待网络空闲
    ///
    /// 轮询检测直到页面 `document.readyState === "complete"` 且在 `idle_duration_ms` 毫秒内
    /// 没有新的资源请求。
    pub async fn wait_for_network_idle(
        &self,
        idle_duration_ms: u64,
        timeout_ms: u64,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let idle_threshold = Duration::from_millis(idle_duration_ms);
        let poll_interval = Duration::from_millis(100);

        let mut last_resource_count: usize = 0;
        let mut stable_since = Instant::now();

        let js = r#"(() => {
            const ready = document.readyState === 'complete';
            const resources = window.performance.getEntriesByType('resource').length;
            return {ready, resources};
        })()"#;

        loop {
            let val = self.evaluate_in_active_context(js).await?;

            let ready = val["ready"].as_bool().unwrap_or(false);
            let resources = val["resources"].as_u64().unwrap_or(0) as usize;

            if resources != last_resource_count {
                last_resource_count = resources;
                stable_since = Instant::now();
            }

            if ready && stable_since.elapsed() >= idle_threshold {
                debug!(
                    "Network idle after {}ms",
                    stable_since.elapsed().as_millis()
                );
                return Ok(());
            }

            if Instant::now() >= deadline {
                return Err(Error::Timeout(format!(
                    "Network did not become idle within {timeout_ms}ms"
                )));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

// ---------------------------------------------------------------------------
// 文件上传
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 文件上传
    ///
    /// 通过 CDP 设置 `<input type="file">` 元素的文件
    ///
    /// # Security
    ///
    /// 验证文件路径，防止路径遍历攻击。
    pub async fn upload_file(&self, ref_id: &str, file_path: &str) -> Result<()> {
        self.upload_file_bound(None, ref_id, file_path).await
    }

    /// Upload a file only when the target belongs to the supplied snapshot.
    pub async fn upload_file_with_snapshot(
        &self,
        snapshot_id: &str,
        ref_id: &str,
        file_path: &str,
    ) -> Result<()> {
        self.validate_snapshot_ref(snapshot_id, Some(ref_id))
            .await?;
        self.upload_file_bound(Some(snapshot_id), ref_id, file_path)
            .await
    }

    async fn upload_file_bound(
        &self,
        snapshot_id: Option<&str>,
        ref_id: &str,
        file_path: &str,
    ) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::dom::{
            GetDocumentParams, QuerySelectorParams, SetFileInputFilesParams,
        };

        // 验证文件路径
        let validated_path = validate_file_path(file_path, &self.config.allowed_file_roots)?;

        let page = self.active_page().await?;
        let selector = match snapshot_id {
            Some(snapshot_id) => {
                format!("[data-agent-ref={ref_id:?}][data-agent-snapshot={snapshot_id:?}]")
            }
            None => format!("[data-agent-ref={ref_id:?}]"),
        };

        // 获取根节点 ID
        let root_node_id = page
            .execute(GetDocumentParams {
                depth: Some(0),
                pierce: Some(false),
            })
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?
            .root
            .node_id;

        // 查找元素节点 ID
        let node_id = page
            .execute(QuerySelectorParams {
                node_id: root_node_id,
                selector: selector.clone(),
            })
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?
            .node_id;

        // 设置文件
        page.execute(SetFileInputFilesParams {
            files: vec![validated_path.to_string_lossy().to_string()],
            node_id: Some(node_id),
            backend_node_id: None,
            object_id: None,
        })
        .await
        .map_err(|e| Error::Cdp(e.to_string()))?;

        info!("Uploaded file: {} -> {}", file_path, ref_id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 对话框处理
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 设置对话框自动处理
    ///
    /// 必须在触发对话框的动作**之前**调用
    /// 可以处理连续弹出的多个对话框（如 alert + confirm）
    pub async fn setup_dialog_handler(
        &self,
        accept: bool,
        prompt_text: Option<String>,
    ) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::page::{
            EventJavascriptDialogOpening, HandleJavaScriptDialogParams,
        };
        use futures::StreamExt;

        let page = self.active_page().await?;

        let mut events = page
            .event_listener::<EventJavascriptDialogOpening>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let page_clone = page.clone();
        let prompt_text = prompt_text.unwrap_or_default();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            // 循环处理所有对话框，不只是第一个
            while let Some(event) = events.next().await {
                debug!("Handling dialog event");
                let _ = event_tx.send(BrowserEvent::DialogOpened {
                    message: event.message.clone(),
                    dialog_type: format!("{:?}", event.r#type),
                });
                let _ = page_clone
                    .execute(HandleJavaScriptDialogParams {
                        accept,
                        prompt_text: Some(prompt_text.clone()),
                    })
                    .await;
            }
            debug!("Dialog handler stream ended");
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// iframe 上下文切换
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 进入 iframe
    ///
    /// 切换到指定 iframe 的上下文，后续操作将在该 iframe 内执行。
    /// 支持嵌套 iframe（可以在 iframe 内再进入子 iframe）。
    ///
    /// # 参数
    ///
    /// - `ref_id`: iframe 元素的 ref_id（如 "iframe1"）
    ///
    /// # 返回
    ///
    /// 返回当前 iframe 栈深度
    pub async fn enter_iframe(&self, ref_id: &str) -> Result<usize> {
        // 首先检查映射表
        let mapping = self.iframe_mapping.lock().await;
        let frame_id = mapping.get(ref_id).cloned();
        drop(mapping);

        let frame_id = if let Some(fid) = frame_id {
            info!("Found frame_id {} for ref_id {} from mapping", fid, ref_id);
            fid
        } else {
            // 映射表中没有，尝试通过 CDP frame tree 查找
            warn!(
                "No mapping found for ref_id {}, searching frame tree",
                ref_id
            );
            return self.enter_iframe_by_search(ref_id).await;
        };

        // 获取该 frame 的执行上下文
        let context_id = self.get_frame_execution_context(&frame_id).await?;
        let (offset_x, offset_y) = self
            .get_frame_viewport_offset(&frame_id)
            .await
            .unwrap_or((0.0, 0.0));

        // 更新 iframe 状态（单一锁，防止竞态条件）
        let mut state = self.iframe_state.lock().await;
        state.active_frame_id = Some(frame_id.clone());
        state.active_context_id = Some(context_id);
        state.iframe_stack.push(IframeContext {
            frame_id: frame_id.clone(),
            url: None,
            offset_x,
            offset_y,
        });
        let depth = state.iframe_stack.len();
        info!(
            "Entered iframe: ref_id={}, frame_id={}, context_id={}, depth={}",
            ref_id, frame_id, context_id, depth
        );
        drop(state);
        *self.snapshot_state.lock().await = None;
        Ok(depth)
    }

    /// Enter an iframe only when its reference belongs to the supplied snapshot.
    pub async fn enter_iframe_with_snapshot(
        &self,
        snapshot_id: &str,
        ref_id: &str,
    ) -> Result<usize> {
        self.validate_snapshot_ref(snapshot_id, Some(ref_id))
            .await?;
        self.enter_iframe(ref_id).await
    }

    /// 通过搜索 frame tree 进入 iframe（备用方案）
    async fn enter_iframe_by_search(&self, ref_id: &str) -> Result<usize> {
        use chromiumoxide::cdp::browser_protocol::page::GetFrameTreeParams;

        let page = self.active_page().await?;

        // 获取 frame tree
        let frame_tree = page
            .execute(GetFrameTreeParams {})
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        // 查找对应的 frame_id
        let frame_id = self
            .find_frame_id_by_ref(&frame_tree.frame_tree, ref_id)
            .await?;

        if let Some(fid) = frame_id {
            // 获取执行上下文
            let context_id = self.get_frame_execution_context(&fid).await?;
            let (offset_x, offset_y) = self
                .get_frame_viewport_offset(&fid)
                .await
                .unwrap_or((0.0, 0.0));

            // 更新 iframe 状态（单一锁）
            let mut state = self.iframe_state.lock().await;
            state.active_frame_id = Some(fid.clone());
            state.active_context_id = Some(context_id);
            state.iframe_stack.push(IframeContext {
                frame_id: fid.clone(),
                url: None,
                offset_x,
                offset_y,
            });
            let depth = state.iframe_stack.len();

            info!(
                "Entered iframe (search): ref_id={}, frame_id={}, depth={}",
                ref_id, fid, depth
            );
            drop(state);
            *self.snapshot_state.lock().await = None;
            Ok(depth)
        } else {
            Err(Error::ElementNotFound(format!(
                "iframe with ref_id={}",
                ref_id
            )))
        }
    }

    /// 获取指定 frame 的执行上下文 ID
    async fn get_frame_execution_context(&self, frame_id: &str) -> Result<i64> {
        use chromiumoxide::cdp::browser_protocol::page::CreateIsolatedWorldParams;

        let page = self.active_page().await?;

        let context = page
            .execute(
                CreateIsolatedWorldParams::builder()
                    .frame_id(frame_id.to_string())
                    .world_name("agent-browser")
                    .grant_univeral_access(false)
                    .build()
                    .map_err(Error::InvalidParameter)?,
            )
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?
            .result
            .execution_context_id;
        let context_id = *context.inner();
        Ok(context_id)
    }

    async fn get_frame_viewport_offset(&self, frame_id: &str) -> Result<(f64, f64)> {
        use chromiumoxide::cdp::browser_protocol::dom::{GetBoxModelParams, GetFrameOwnerParams};

        let page = self.active_page().await?;
        let owner = page
            .execute(GetFrameOwnerParams::new(frame_id.to_string()))
            .await
            .map_err(|error| Error::Cdp(error.to_string()))?;
        let model = page
            .execute(GetBoxModelParams {
                node_id: owner.node_id,
                backend_node_id: Some(owner.backend_node_id),
                object_id: None,
            })
            .await
            .map_err(|error| Error::Cdp(error.to_string()))?
            .model
            .clone();
        let quad = model.content.inner();
        Ok((
            quad.first().copied().unwrap_or_default(),
            quad.get(1).copied().unwrap_or_default(),
        ))
    }

    /// 退出 iframe
    ///
    /// 退出当前 iframe 上下文，返回到父级上下文。
    /// 如果已经处于主文档上下文，则不做任何操作。
    ///
    /// # 返回
    ///
    /// 返回当前 iframe 栈深度
    pub async fn exit_iframe(&self) -> Result<usize> {
        // 第一步：弹出 iframe 并获取父 frame 信息（持锁时间短）
        let (parent_frame_id, depth) = {
            let mut state = self.iframe_state.lock().await;
            let popped = state.iframe_stack.pop();
            let parent = state.iframe_stack.last().map(|c| c.frame_id.clone());
            let current_depth = state.iframe_stack.len();

            if let Some(ref ctx) = popped {
                info!(
                    "Exited iframe: frame_id={}, depth={}",
                    ctx.frame_id, current_depth
                );
            }
            (parent, current_depth)
        };
        // 锁已释放

        // 第二步：获取父 frame 的执行上下文（不持锁）
        let parent_context_id = if let Some(ref parent_fid) = parent_frame_id {
            self.get_frame_execution_context(parent_fid).await.ok()
        } else {
            None
        };

        // 第三步：更新 iframe 状态（重新获取锁）
        {
            let mut state = self.iframe_state.lock().await;
            state.active_frame_id = parent_frame_id;
            state.active_context_id = parent_context_id;
        }
        *self.snapshot_state.lock().await = None;
        Ok(depth)
    }

    /// 退出所有 iframe
    ///
    /// 清空 iframe 栈，返回到主文档上下文。
    pub async fn exit_all_iframes(&self) -> Result<()> {
        // 单一锁更新所有 iframe 状态
        let mut state = self.iframe_state.lock().await;
        state.iframe_stack.clear();
        state.active_frame_id = None;
        state.active_context_id = None;
        info!("Exited all iframes");
        drop(state);
        *self.snapshot_state.lock().await = None;
        Ok(())
    }

    /// 获取当前 iframe 深度
    pub async fn iframe_depth(&self) -> usize {
        self.iframe_state.lock().await.iframe_stack.len()
    }

    /// 在当前上下文中执行 JavaScript
    ///
    /// 如果当前在 iframe 上下文中，将尝试在该 iframe 内执行脚本。
    pub async fn evaluate_in_context(&self, script: &str) -> Result<serde_json::Value> {
        self.evaluate_in_active_context(script).await
    }

    /// 获取当前 iframe 的快照
    ///
    /// 如果当前在 iframe 上下文中，将获取该 iframe 内的元素快照。
    pub async fn snapshot_in_frame(&self) -> Result<PageSnapshot> {
        use chromiumoxide::cdp::browser_protocol::accessibility::{
            EnableParams, GetFullAxTreeParams,
        };

        let page = self.active_page().await?;
        let active_frame = self.iframe_state.lock().await.active_frame_id.clone();
        let snapshot_id = uuid::Uuid::new_v4().to_string();

        // 启用 Accessibility 域
        let _ = page.execute(EnableParams {}).await;

        let frame_id =
            active_frame.map(|f| chromiumoxide::cdp::browser_protocol::page::FrameId::new(&f));

        let ax_result = page
            .execute(GetFullAxTreeParams {
                depth: None,
                frame_id,
            })
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let ax_nodes = ax_result.nodes.clone();

        if ax_nodes.is_empty() {
            return Ok(PageSnapshot {
                snapshot_id,
                url: String::new(),
                title: String::new(),
                nodes: Vec::new(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                iframe_count: 0,
                iframe_mappings: Vec::new(),
            });
        }

        // 处理节点
        let (nodes, _) =
            crate::snapshot::process_ax_nodes_in_frame(&page, &ax_nodes, 0, &snapshot_id).await?;

        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        Ok(PageSnapshot {
            snapshot_id,
            url,
            title,
            nodes,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            iframe_count: 0,
            iframe_mappings: Vec::new(),
        })
    }

    /// 在 iframe 内查找元素
    async fn find_frame_id_by_ref(
        &self,
        frame_tree: &chromiumoxide::cdp::browser_protocol::page::FrameTree,
        ref_id: &str,
    ) -> Result<Option<String>> {
        // 使用 JavaScript 查找 iframe 的 frame_id
        let page = self.active_page().await?;

        let js = format!(
            r#"(function() {{
                const iframes = document.querySelectorAll('iframe');
                for (const iframe of iframes) {{
                    const ref = iframe.getAttribute('data-agent-ref');
                    if (ref === '{}' || iframe.src && iframe.src.includes('{}')) {{
                        // 返回 iframe 的 name 或 id 作为标识
                        return iframe.name || iframe.id || iframe.src;
                    }}
                }}
                return null;
            }})()"#,
            ref_id, ref_id
        );

        let _result: Option<String> = page
            .evaluate(js.as_str())
            .await
            .ok()
            .and_then(|v| v.into_value().ok());

        // 同时从 CDP frame tree 中查找
        Ok(self.find_frame_in_tree(frame_tree, ref_id))
    }

    /// 递归查找 frame
    fn find_frame_in_tree(
        &self,
        frame_tree: &chromiumoxide::cdp::browser_protocol::page::FrameTree,
        ref_id: &str,
    ) -> Option<String> {
        // 简化实现：通过 URL 或 name 匹配
        if let Some(children) = &frame_tree.child_frames {
            for child in children {
                let frame_id: String = child.frame.id.clone().into();
                // 尝试匹配
                if frame_id.contains(ref_id) {
                    return Some(frame_id);
                }
                // 递归查找
                if let Some(found) = self.find_frame_in_tree(child, ref_id) {
                    return Some(found);
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// 文件下载
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 设置下载行为
    ///
    /// 配置浏览器的下载目录，并启用下载事件监听。
    /// 必须在触发下载操作**之前**调用。
    ///
    /// # Security
    ///
    /// 验证下载路径，防止路径遍历攻击。
    pub async fn setup_download(&self, save_path: Option<&str>) -> Result<PathBuf> {
        use chromiumoxide::cdp::browser_protocol::browser::SetDownloadBehaviorParams;

        let page = self.active_page().await?;

        // 确定下载目录（验证自定义路径）
        let download_dir = if let Some(path) = save_path {
            validate_directory_path(path, &self.config.allowed_file_roots)?
        } else {
            // 默认使用临时目录
            std::env::temp_dir().join("echo-browser-downloads")
        };

        // 创建目录
        tokio::fs::create_dir_all(&download_dir).await?;

        let dir_str = download_dir.to_string_lossy().to_string();

        // 设置下载行为
        page.execute(SetDownloadBehaviorParams {
            behavior:
                chromiumoxide::cdp::browser_protocol::browser::SetDownloadBehaviorBehavior::Allow,
            download_path: Some(dir_str.clone()),
            events_enabled: Some(true),
            browser_context_id: None,
        })
        .await
        .map_err(|e| Error::Cdp(e.to_string()))?;

        // 保存下载目录
        *self.download_dir.lock().await = Some(download_dir.clone());

        info!("Download behavior set: {}", dir_str);
        Ok(download_dir)
    }

    /// 等待下载完成
    ///
    /// 等待指定的下载完成，返回下载结果。
    /// 如果不指定 guid，则等待下一个下载完成。
    pub async fn wait_for_download(
        &self,
        guid: Option<&str>,
        timeout_ms: u64,
    ) -> Result<DownloadResult> {
        let listeners = self.create_download_listeners().await?;
        self.wait_for_download_with_listeners(listeners, guid, timeout_ms)
            .await
    }

    async fn create_download_listeners(&self) -> Result<DownloadListeners> {
        use chromiumoxide::cdp::browser_protocol::browser::{
            EventDownloadProgress, EventDownloadWillBegin,
        };

        let page = self.active_page().await?;
        let begin = page
            .event_listener::<EventDownloadWillBegin>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;
        let progress = page
            .event_listener::<EventDownloadProgress>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;
        Ok(DownloadListeners { begin, progress })
    }

    async fn wait_for_download_with_listeners(
        &self,
        mut listeners: DownloadListeners,
        guid: Option<&str>,
        timeout_ms: u64,
    ) -> Result<DownloadResult> {
        use chromiumoxide::cdp::browser_protocol::browser::DownloadProgressState;
        use futures::StreamExt;

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut downloads: HashMap<String, String> = HashMap::new();

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("Download wait timeout".to_string()));
            }

            let event = tokio::time::timeout(remaining, async {
                tokio::select! {
                    event = listeners.begin.next() => event.map(DownloadEvent::Begin),
                    event = listeners.progress.next() => event.map(DownloadEvent::Progress),
                }
            })
            .await
            .map_err(|_| Error::Timeout("Download wait timeout".to_string()))?
            .ok_or_else(|| Error::Cdp("Download event stream closed".to_string()))?;

            match event {
                DownloadEvent::Begin(event) => {
                    downloads.insert(event.guid.clone(), event.suggested_filename.clone());
                }
                DownloadEvent::Progress(event) => {
                    if guid.is_some_and(|expected| event.guid != expected) {
                        continue;
                    }
                    match event.state {
                        DownloadProgressState::InProgress => {
                            debug!(
                                "Download in progress: {} ({} bytes)",
                                event.guid, event.received_bytes
                            );
                        }
                        DownloadProgressState::Canceled => {
                            return Err(Error::Other(format!("Download canceled: {}", event.guid)));
                        }
                        DownloadProgressState::Completed => {
                            let filename = downloads
                                .get(&event.guid)
                                .cloned()
                                .or_else(|| {
                                    event.file_path.as_ref().and_then(|path| {
                                        Path::new(path)
                                            .file_name()
                                            .map(|name| name.to_string_lossy().into_owned())
                                    })
                                })
                                .unwrap_or_else(|| format!("download-{}", event.guid));
                            let download_dir = self.download_dir.lock().await.clone();
                            let file_path = event
                                .file_path
                                .as_ref()
                                .map(PathBuf::from)
                                .or_else(|| download_dir.map(|dir| dir.join(&filename)));
                            let file_path = file_path.ok_or_else(|| {
                                Error::Other("Download completed without a file path".to_string())
                            })?;
                            let result = DownloadResult {
                                guid: event.guid.clone(),
                                filename,
                                file_path: file_path.to_string_lossy().into_owned(),
                                size: Some(event.received_bytes as u64),
                                mime_type: None,
                                status: DownloadStatus::Completed,
                            };
                            let _ = self.download_events.send(result.clone());
                            let _ = self
                                .event_tx
                                .send(BrowserEvent::DownloadCompleted(result.clone()));
                            info!("Download completed: {} -> {:?}", event.guid, file_path);
                            return Ok(result);
                        }
                    }
                }
            }
        }
    }

    /// 下载文件
    ///
    /// 直接下载指定 URL 的文件。
    pub async fn download_file(
        &self,
        url: &str,
        options: Option<DownloadOptions>,
    ) -> Result<DownloadResult> {
        let opts = options.unwrap_or_default();
        let timeout = opts.timeout_ms.unwrap_or(60000);

        // 设置下载目录
        let _download_dir = self.setup_download(opts.save_path.as_deref()).await?;
        let listeners = self.create_download_listeners().await?;

        // 记录当前 URL
        let page = self.active_page().await?;

        // 触发下载：使用 JavaScript 创建隐藏的下载链接（URL 使用 {:?} 正确转义）
        let js = format!(
            r#"
            (function() {{
                const a = document.createElement('a');
                a.href = {url:?};
                a.download = '';
                a.style.display = 'none';
                document.body.appendChild(a);
                a.click();
                document.body.removeChild(a);
                return true;
            }})()
            "#,
            url = url
        );

        page.evaluate(js.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        info!("Download triggered: {}", url);

        // 等待下载完成
        self.wait_for_download_with_listeners(listeners, None, timeout)
            .await
    }

    /// 点击元素并等待下载
    ///
    /// 点击指定元素后等待下载完成。
    pub async fn click_and_download(
        &self,
        ref_id: &str,
        options: Option<DownloadOptions>,
    ) -> Result<DownloadResult> {
        self.click_and_download_bound(None, ref_id, options).await
    }

    /// Click a snapshot-bound element and wait for its download.
    pub async fn click_and_download_with_snapshot(
        &self,
        snapshot_id: &str,
        ref_id: &str,
        options: Option<DownloadOptions>,
    ) -> Result<DownloadResult> {
        self.click_and_download_bound(Some(snapshot_id), ref_id, options)
            .await
    }

    async fn click_and_download_bound(
        &self,
        snapshot_id: Option<&str>,
        ref_id: &str,
        options: Option<DownloadOptions>,
    ) -> Result<DownloadResult> {
        let opts = options.unwrap_or_default();
        let timeout = opts.timeout_ms.unwrap_or(60000);

        // 设置下载目录
        self.setup_download(opts.save_path.as_deref()).await?;
        let listeners = self.create_download_listeners().await?;

        // 点击元素
        if let Some(snapshot_id) = snapshot_id {
            self.act_with_snapshot(snapshot_id, ref_id, ActionKind::Click)
                .await?;
        } else {
            self.click(ref_id).await?;
        }

        info!("Clicked element {} for download", ref_id);

        // 等待下载完成
        self.wait_for_download_with_listeners(listeners, None, timeout)
            .await
    }

    /// 获取下载目录
    pub async fn get_download_dir(&self) -> Option<PathBuf> {
        self.download_dir.lock().await.clone()
    }
}

// ---------------------------------------------------------------------------
// 键盘组合键
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 按键（支持修饰键）
    ///
    /// 执行键盘按键操作，支持 Ctrl、Shift、Alt、Meta 等修饰键。
    pub async fn press_with_modifiers(
        &self,
        key: &str,
        modifiers: &[KeyModifier],
    ) -> Result<ActionResult> {
        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchKeyEventParams, DispatchKeyEventType,
        };

        let page = self.active_page().await?;

        // 转换修饰键
        let modifier_flags: u8 = modifiers.iter().fold(0u8, |acc, m| {
            acc | match m {
                KeyModifier::Alt => 1,
                KeyModifier::Control => 2,
                KeyModifier::Shift => 8,
                KeyModifier::Meta => 4,
            }
        });

        // 规范化按键名称
        let key_normalized = normalize_key(key);
        let code = key_to_code(&key_normalized);
        let key_text = if key_normalized.len() == 1 {
            Some(key_normalized.clone())
        } else {
            None
        };

        info!(
            "Pressing key: {} with modifiers {:?}",
            key_normalized, modifiers
        );

        // 按下
        page.execute(DispatchKeyEventParams {
            r#type: DispatchKeyEventType::KeyDown,
            key: Some(key_normalized.clone()),
            code: Some(code.clone()),
            text: key_text.clone(),
            modifiers: Some(modifier_flags as i64),
            windows_virtual_key_code: None,
            native_virtual_key_code: None,
            auto_repeat: None,
            is_keypad: None,
            is_system_key: None,
            location: None,
            commands: None,
            key_identifier: None,
            timestamp: None,
            unmodified_text: None,
        })
        .await
        .map_err(|e| Error::Cdp(e.to_string()))?;

        // 释放
        page.execute(DispatchKeyEventParams {
            r#type: DispatchKeyEventType::KeyUp,
            key: Some(key_normalized.clone()),
            code: Some(code),
            text: key_text,
            modifiers: Some(modifier_flags as i64),
            windows_virtual_key_code: None,
            native_virtual_key_code: None,
            auto_repeat: None,
            is_keypad: None,
            is_system_key: None,
            location: None,
            commands: None,
            key_identifier: None,
            timestamp: None,
            unmodified_text: None,
        })
        .await
        .map_err(|e| Error::Cdp(e.to_string()))?;

        Ok(ActionResult {
            success: true,
            message: format!("Pressed {} with modifiers {:?}", key_normalized, modifiers),
        })
    }

    /// 发送键盘快捷键
    ///
    /// 便捷方法，用于发送常见的键盘快捷键。
    pub async fn send_shortcut(&self, shortcut: &str) -> Result<ActionResult> {
        // 检测操作系统，Mac 使用 Meta，其他使用 Control
        #[cfg(target_os = "macos")]
        let main_modifier = KeyModifier::Meta;
        #[cfg(not(target_os = "macos"))]
        let main_modifier = KeyModifier::Control;

        let (key, modifiers): (&str, Vec<KeyModifier>) = match shortcut.to_lowercase().as_str() {
            "copy" => ("c", vec![main_modifier.clone()]),
            "paste" => ("v", vec![main_modifier.clone()]),
            "cut" => ("x", vec![main_modifier.clone()]),
            "save" => ("s", vec![main_modifier.clone()]),
            "selectall" | "select_all" => ("a", vec![main_modifier.clone()]),
            "undo" => ("z", vec![main_modifier.clone()]),
            "redo" => ("z", vec![main_modifier.clone(), KeyModifier::Shift]),
            "find" => ("f", vec![main_modifier.clone()]),
            "refresh" => ("F5", vec![]),
            "devtools" => ("i", vec![main_modifier.clone(), KeyModifier::Shift]),
            "print" => ("p", vec![main_modifier.clone()]),
            "newtab" | "new_tab" => ("t", vec![main_modifier.clone()]),
            "closetab" | "close_tab" => ("w", vec![main_modifier.clone()]),
            other => {
                return Err(Error::InvalidParameter(format!(
                    "Unknown shortcut: {}",
                    other
                )));
            }
        };

        self.press_with_modifiers(key, &modifiers).await
    }

    /// 输入文本（逐字符）
    ///
    /// 逐字符输入文本，模拟真实键盘输入。
    pub async fn type_text_simulated(&self, text: &str) -> Result<ActionResult> {
        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchKeyEventParams, DispatchKeyEventType,
        };

        let page = self.active_page().await?;

        for ch in text.chars() {
            let key = ch.to_string();

            // 按下
            page.execute(DispatchKeyEventParams {
                r#type: DispatchKeyEventType::KeyDown,
                key: Some(key.clone()),
                code: Some(format!("Key{}", key.to_uppercase())),
                text: Some(key.clone()),
                modifiers: None,
                windows_virtual_key_code: None,
                native_virtual_key_code: None,
                auto_repeat: None,
                is_keypad: None,
                is_system_key: None,
                location: None,
                commands: None,
                key_identifier: None,
                timestamp: None,
                unmodified_text: None,
            })
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

            // 释放
            page.execute(DispatchKeyEventParams {
                r#type: DispatchKeyEventType::KeyUp,
                key: Some(key.clone()),
                code: Some(format!("Key{}", key.to_uppercase())),
                text: Some(key),
                modifiers: None,
                windows_virtual_key_code: None,
                native_virtual_key_code: None,
                auto_repeat: None,
                is_keypad: None,
                is_system_key: None,
                location: None,
                commands: None,
                key_identifier: None,
                timestamp: None,
                unmodified_text: None,
            })
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

            // 小延迟
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(ActionResult {
            success: true,
            message: format!("Typed {} characters", text.len()),
        })
    }
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 规范化按键名称
fn normalize_key(key: &str) -> String {
    match key.to_lowercase().as_str() {
        "enter" | "return" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        "backspace" | "back" => "Backspace".to_string(),
        "delete" | "del" => "Delete".to_string(),
        "arrowup" | "up" => "ArrowUp".to_string(),
        "arrowdown" | "down" => "ArrowDown".to_string(),
        "arrowleft" | "left" => "ArrowLeft".to_string(),
        "arrowright" | "right" => "ArrowRight".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        "space" => " ".to_string(),
        "f1" => "F1".to_string(),
        "f2" => "F2".to_string(),
        "f3" => "F3".to_string(),
        "f4" => "F4".to_string(),
        "f5" => "F5".to_string(),
        "f6" => "F6".to_string(),
        "f7" => "F7".to_string(),
        "f8" => "F8".to_string(),
        "f9" => "F9".to_string(),
        "f10" => "F10".to_string(),
        "f11" => "F11".to_string(),
        "f12" => "F12".to_string(),
        other if other.len() == 1 => other.to_uppercase(),
        other => other.to_string(),
    }
}

/// 转换按键到代码
fn key_to_code(key: &str) -> String {
    match key {
        "Enter" => "Enter".to_string(),
        "Tab" => "Tab".to_string(),
        "Escape" => "Escape".to_string(),
        "Backspace" => "Backspace".to_string(),
        "Delete" => "Delete".to_string(),
        "ArrowUp" => "ArrowUp".to_string(),
        "ArrowDown" => "ArrowDown".to_string(),
        "ArrowLeft" => "ArrowLeft".to_string(),
        "ArrowRight" => "ArrowRight".to_string(),
        "Home" => "Home".to_string(),
        "End" => "End".to_string(),
        "PageUp" => "PageUp".to_string(),
        "PageDown" => "PageDown".to_string(),
        " " => "Space".to_string(),
        k if k.starts_with('F') && k.len() <= 3 => k.to_string(),
        k if k.len() == 1 => format!("Key{}", k.to_uppercase()),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Network 监听 / 拦截
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 启用 Network 域，开始监听网络事件
    pub async fn enable_network(&self) -> Result<()> {
        let page = self.active_page().await?;
        use chromiumoxide::cdp::browser_protocol::network::EnableParams;
        page.execute(EnableParams::default())
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;
        debug!("Network domain enabled");
        Ok(())
    }

    /// 监听所有网络请求，收集为 NetworkRequest 列表
    ///
    /// 在指定的时间窗口内收集所有网络请求事件。
    /// # 参数
    /// - `duration_ms`: 监听持续时间（毫秒）
    pub async fn listen_network_requests(
        &self,
        duration_ms: u64,
    ) -> Result<Vec<crate::types::NetworkRequest>> {
        use chromiumoxide::cdp::browser_protocol::network::EventRequestWillBeSent;
        use futures::StreamExt;

        let page = self.active_page().await?;

        // 确保启用了 Network 域
        self.enable_network().await?;

        let mut stream = page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let mut requests = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(duration_ms);

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, stream.next()).await {
                Ok(Some(event)) => {
                    let resource_type = event
                        .r#type
                        .as_ref()
                        .map(|t| t.as_ref().to_string())
                        .unwrap_or_default();
                    requests.push(crate::types::NetworkRequest {
                        request_id: Into::<String>::into(event.request_id.clone()),
                        url: event.request.url.clone(),
                        method: event.request.method.clone(),
                        resource_type,
                        headers: sanitize_headers(
                            event.request.headers.inner().clone(),
                            self.config.capture_sensitive_data,
                        ),
                        post_data: self
                            .config
                            .capture_sensitive_data
                            .then(|| {
                                event
                                    .request
                                    .post_data_entries
                                    .as_ref()
                                    .and_then(|entries| entries.first())
                                    .and_then(|entry| entry.bytes.clone())
                                    .map(String::from)
                            })
                            .flatten(),
                    });
                }
                Ok(None) => break,
                Err(_) => break, // timeout
            }
        }

        info!(
            "Collected {} network requests in {}ms",
            requests.len(),
            duration_ms
        );
        Ok(requests)
    }

    /// 监听所有网络响应，收集为 NetworkResponse 列表
    ///
    /// # 参数
    /// - `duration_ms`: 监听持续时间（毫秒）
    pub async fn listen_network_responses(
        &self,
        duration_ms: u64,
    ) -> Result<Vec<crate::types::NetworkResponse>> {
        use chromiumoxide::cdp::browser_protocol::network::EventResponseReceived;
        use futures::StreamExt;

        let page = self.active_page().await?;
        self.enable_network().await?;

        let mut stream = page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let mut responses = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(duration_ms);

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, stream.next()).await {
                Ok(Some(event)) => {
                    responses.push(crate::types::NetworkResponse {
                        request_id: Into::<String>::into(event.request_id.clone()),
                        url: event.response.url.clone(),
                        status: event.response.status as i32,
                        status_text: event.response.status_text.clone(),
                        headers: sanitize_headers(
                            event.response.headers.inner().clone(),
                            self.config.capture_sensitive_data,
                        ),
                        mime_type: Some(event.response.mime_type.clone()),
                        blocked: false,
                    });
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        info!(
            "Collected {} network responses in {}ms",
            responses.len(),
            duration_ms
        );
        Ok(responses)
    }

    /// 获取指定请求的响应体
    ///
    /// # 参数
    /// - `request_id`: 请求 ID（从 listen_network_responses 获取）
    pub async fn get_response_body(&self, request_id: &str) -> Result<String> {
        use chromiumoxide::cdp::browser_protocol::network::{GetResponseBodyParams, RequestId};

        let page = self.active_page().await?;
        let params = GetResponseBodyParams::new(RequestId::from(request_id.to_string()));
        let result = page
            .execute(params)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        Ok(result.body.clone())
    }

    /// 拦截并修改请求（使用 Fetch domain）
    ///
    /// 启用请求拦截，匹配 URL 模式的请求将被暂停，可通过回调修改后继续。
    /// # 参数
    /// - `url_pattern`: URL 匹配模式（如 "*" 匹配所有请求）
    /// - `block`: 是否阻止匹配的请求
    pub async fn intercept_requests(&self, url_pattern: &str, block: bool) -> Result<()> {
        if url_pattern.trim().is_empty() {
            return Err(Error::InvalidParameter(
                "URL interception pattern cannot be empty".to_string(),
            ));
        }
        let page = self.active_page().await?;
        self.ensure_network_policy(&page).await?;
        let mut patterns = self.blocked_url_patterns.lock().await;
        if block {
            if !patterns.iter().any(|pattern| pattern == url_pattern) {
                patterns.push(url_pattern.to_string());
            }
        } else {
            patterns.retain(|pattern| pattern != url_pattern);
        }

        info!(
            "Request block rule updated for pattern: {} (block: {})",
            url_pattern, block
        );
        Ok(())
    }

    /// 禁用请求拦截
    pub async fn disable_interception(&self) -> Result<()> {
        self.blocked_url_patterns.lock().await.clear();
        info!("Runtime request block rules cleared");
        Ok(())
    }

    /// List active runtime request block rules.
    pub async fn blocked_request_patterns(&self) -> Vec<String> {
        self.blocked_url_patterns.lock().await.clone()
    }
}

// ---------------------------------------------------------------------------
// Console 消息监听
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 启用 Runtime 域并开始监听 Console 消息
    ///
    /// 在指定的时间窗口内收集所有 console 输出。
    /// # 参数
    /// - `duration_ms`: 监听持续时间（毫秒）
    pub async fn listen_console(
        &self,
        duration_ms: u64,
    ) -> Result<Vec<crate::types::ConsoleMessage>> {
        use chromiumoxide::cdp::js_protocol::runtime::EventConsoleApiCalled;
        use futures::StreamExt;

        let page = self.active_page().await?;

        // 启用 Runtime 域
        use chromiumoxide::cdp::js_protocol::runtime::EnableParams;
        page.execute(EnableParams::default())
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let mut stream = page
            .event_listener::<EventConsoleApiCalled>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let mut messages = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(duration_ms);

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, stream.next()).await {
                Ok(Some(event)) => {
                    let level = event.r#type.as_ref().to_string();
                    let text: String = event
                        .args
                        .iter()
                        .filter_map(|v| v.value.as_ref())
                        .filter_map(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .or_else(|| Some(v.to_string()))
                        })
                        .collect::<Vec<_>>()
                        .join(" ");

                    messages.push(crate::types::ConsoleMessage {
                        level,
                        text,
                        url: None,
                        line_number: None,
                        timestamp: serde_json::to_value(&event.timestamp)
                            .ok()
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                    });
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        info!(
            "Collected {} console messages in {}ms",
            messages.len(),
            duration_ms
        );
        Ok(messages)
    }
}

// ---------------------------------------------------------------------------
// 语义化 Locator（getByRole / getByText / getByLabel）
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 通过 ARIA role 查找元素并点击
    ///
    /// 使用 Accessibility 树查找具有指定 role 的元素。
    /// # 参数
    /// - `role`: ARIA role（如 "button", "link", "textbox", "checkbox" 等）
    /// - `name`: 可选的 accessible name 过滤
    /// - `timeout_ms`: 等待超时
    pub async fn click_by_role(
        &self,
        role: &str,
        name: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let ref_id = self.find_by_role(role, name, timeout_ms).await?;
        self.click(&ref_id).await
    }

    /// 通过文本内容查找元素并点击
    ///
    /// 在快照中查找包含指定文本的元素。
    /// # 参数
    /// - `text`: 要匹配的文本内容
    /// - `timeout_ms`: 等待超时
    pub async fn click_by_text(&self, text: &str, timeout_ms: Option<u64>) -> Result<ActionResult> {
        let ref_id = self.find_by_text(text, timeout_ms).await?;
        self.click(&ref_id).await
    }

    /// 通过 label 文本查找元素并点击
    ///
    /// 查找具有匹配 label 的表单元素。
    /// # 参数
    /// - `label`: label 文本
    /// - `timeout_ms`: 等待超时
    pub async fn click_by_label(
        &self,
        label: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let ref_id = self.find_by_label(label, timeout_ms).await?;
        self.click(&ref_id).await
    }

    /// 通过 ARIA role 查找元素并输入文本
    pub async fn type_by_role(
        &self,
        role: &str,
        name: Option<&str>,
        text: &str,
        clear_first: bool,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let ref_id = self.find_by_role(role, name, timeout_ms).await?;
        self.type_text(&ref_id, text, clear_first).await
    }

    /// 通过 label 查找元素并输入文本
    pub async fn type_by_label(
        &self,
        label: &str,
        text: &str,
        clear_first: bool,
        timeout_ms: Option<u64>,
    ) -> Result<ActionResult> {
        let ref_id = self.find_by_label(label, timeout_ms).await?;
        self.type_text(&ref_id, text, clear_first).await
    }

    /// 通过 ARIA role 查找元素的 ref_id
    pub async fn find_by_role(
        &self,
        role: &str,
        name: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<String> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);
        let deadline = Instant::now() + Duration::from_millis(timeout);

        loop {
            let snapshot = self.snapshot().await?;
            let mut nodes = Vec::new();
            collect_snapshot_nodes(&snapshot.nodes, &mut nodes);
            for node in nodes {
                if node.role == role {
                    if let Some(n) = name {
                        if node.name.contains(n) {
                            return Ok(node.ref_id.clone());
                        }
                    } else {
                        return Ok(node.ref_id.clone());
                    }
                }
            }

            if Instant::now() >= deadline {
                return Err(Error::ElementNotFound(format!(
                    "No element with role '{}'{} found",
                    role,
                    name.map(|n| format!(" and name containing '{}'", n))
                        .unwrap_or_default()
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 通过文本内容查找元素的 ref_id
    pub async fn find_by_text(&self, text: &str, timeout_ms: Option<u64>) -> Result<String> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);
        let deadline = Instant::now() + Duration::from_millis(timeout);

        loop {
            let snapshot = self.snapshot().await?;
            let mut nodes = Vec::new();
            collect_snapshot_nodes(&snapshot.nodes, &mut nodes);
            for node in &nodes {
                if node.name.contains(text)
                    || node
                        .value
                        .as_ref()
                        .is_some_and(|value| value.contains(text))
                {
                    // 优先匹配可交互元素
                    let interactive_roles = [
                        "button", "link", "menuitem", "tab", "option", "radio", "checkbox",
                    ];
                    if interactive_roles.contains(&node.role.as_str()) {
                        return Ok(node.ref_id.clone());
                    }
                }
            }
            // 如果没有找到可交互元素，再找任何包含文本的元素
            for node in nodes {
                if node.name.contains(text)
                    || node
                        .value
                        .as_ref()
                        .is_some_and(|value| value.contains(text))
                {
                    return Ok(node.ref_id.clone());
                }
            }

            if Instant::now() >= deadline {
                return Err(Error::ElementNotFound(format!(
                    "No element with text '{}' found",
                    text
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 通过 label 查找元素的 ref_id
    pub async fn find_by_label(&self, label: &str, timeout_ms: Option<u64>) -> Result<String> {
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);
        let deadline = Instant::now() + Duration::from_millis(timeout);

        loop {
            let snapshot = self.snapshot().await?;
            let mut nodes = Vec::new();
            collect_snapshot_nodes(&snapshot.nodes, &mut nodes);
            let form_roles = [
                "textbox",
                "searchbox",
                "combobox",
                "checkbox",
                "radio",
                "slider",
                "spinbutton",
            ];

            for node in nodes {
                if form_roles.contains(&node.role.as_str()) && node.name.contains(label) {
                    return Ok(node.ref_id.clone());
                }
                if node.role == "label"
                    && node.name.contains(label)
                    && let Some(child) = node
                        .children
                        .iter()
                        .find(|child| form_roles.contains(&child.role.as_str()))
                {
                    return Ok(child.ref_id.clone());
                }
                if node
                    .attributes
                    .get("aria-label")
                    .is_some_and(|value| value.contains(label))
                    || node
                        .attributes
                        .get("title")
                        .is_some_and(|value| value.contains(label))
                {
                    return Ok(node.ref_id.clone());
                }
            }

            if Instant::now() >= deadline {
                return Err(Error::ElementNotFound(format!(
                    "No element with label '{}' found",
                    label
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// 运行时视口大小调整
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 设置视口大小
    ///
    /// 使用 CDP Emulation.setDeviceMetricsOverride 动态调整视口大小。
    pub async fn set_viewport_size(&self, width: u32, height: u32) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;

        let page = self.active_page().await?;
        let params = SetDeviceMetricsOverrideParams {
            width: width as i64,
            height: height as i64,
            device_scale_factor: 1.0,
            mobile: false,
            scale: None,
            screen_width: None,
            screen_height: None,
            position_x: None,
            position_y: None,
            dont_set_visible_size: None,
            screen_orientation: None,
            viewport: None,
        };

        page.execute(params)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        info!("Viewport set to {}x{}", width, height);
        Ok(())
    }

    /// 设置视口大小（带设备缩放因子）
    pub async fn set_viewport(&self, viewport: &crate::types::ViewportSize) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;

        let page = self.active_page().await?;
        let params = SetDeviceMetricsOverrideParams {
            width: viewport.width as i64,
            height: viewport.height as i64,
            device_scale_factor: viewport.device_scale_factor.unwrap_or(1.0),
            mobile: false,
            scale: None,
            screen_width: None,
            screen_height: None,
            position_x: None,
            position_y: None,
            dont_set_visible_size: None,
            screen_orientation: None,
            viewport: None,
        };

        page.execute(params)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        info!(
            "Viewport set to {}x{} (scale: {:?})",
            viewport.width, viewport.height, viewport.device_scale_factor
        );
        Ok(())
    }

    /// 获取当前视口大小
    pub async fn get_viewport_size(&self) -> Result<crate::types::ViewportSize> {
        let page = self.active_page().await?;

        let script = r#"({
            width: window.innerWidth,
            height: window.innerHeight,
            devicePixelRatio: window.devicePixelRatio
        })"#;

        let result: serde_json::Value = page
            .evaluate(script)
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        Ok(crate::types::ViewportSize {
            width: result["width"].as_u64().unwrap_or(1920) as u32,
            height: result["height"].as_u64().unwrap_or(1080) as u32,
            device_scale_factor: result["devicePixelRatio"].as_f64(),
        })
    }

    /// 模拟移动设备视口
    pub async fn emulate_device(&self, device_name: &str) -> Result<()> {
        let (width, height, scale, mobile) = match device_name.to_lowercase().as_str() {
            "iphone" | "iphone 14" => (390, 844, 3.0, true),
            "iphone se" => (375, 667, 2.0, true),
            "ipad" | "ipad pro" => (1024, 1366, 2.0, true),
            "pixel" | "pixel 7" => (412, 915, 2.625, true),
            "galaxy" | "galaxy s21" => (360, 800, 3.0, true),
            _ => {
                return Err(Error::InvalidParameter(format!(
                    "Unknown device: {}",
                    device_name
                )));
            }
        };

        use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
        let page = self.active_page().await?;
        let params = SetDeviceMetricsOverrideParams {
            width: width as i64,
            height: height as i64,
            device_scale_factor: scale,
            mobile,
            scale: None,
            screen_width: None,
            screen_height: None,
            position_x: None,
            position_y: None,
            dont_set_visible_size: None,
            screen_orientation: None,
            viewport: None,
        };

        page.execute(params)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        info!("Emulating device: {} ({}x{})", device_name, width, height);
        Ok(())
    }

    /// 重置视口为桌面默认值
    pub async fn reset_viewport(&self) -> Result<()> {
        self.set_viewport_size(1920, 1080).await
    }
}

// ---------------------------------------------------------------------------
// Drop trait 实现（自动清理浏览器进程）
// ---------------------------------------------------------------------------

impl Drop for BrowserEngine {
    fn drop(&mut self) {
        // 尝试优雅关闭浏览器
        // 由于 Drop 不能是 async，使用 block_on 来执行清理
        let browser = self
            .browser
            .try_lock()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(browser) = browser {
            info!("BrowserEngine dropped, cleaning up browser process");
            // 使用 std::thread 在后台尝试关闭
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                if let Ok(rt) = rt {
                    rt.block_on(async {
                        // 关闭所有页面
                        if let Ok(pages) = browser.pages().await {
                            for page in pages {
                                let _ = page.close().await;
                            }
                        }
                        // 关闭浏览器
                        match Arc::try_unwrap(browser) {
                            Ok(mut b) => {
                                let _ = b.close().await;
                            }
                            Err(arc) => {
                                info!(
                                    "Browser has {} remaining references",
                                    Arc::strong_count(&arc)
                                );
                            }
                        }
                    });
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 网络监控
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 启用网络监控
    ///
    /// 启用 CDP Network 域，捕获所有网络请求。
    /// 收集的请求可通过 `get_network_requests()` 获取。
    pub async fn enable_network_monitoring(&self) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::network::{EnableParams, EventRequestWillBeSent};
        use futures::StreamExt;

        let page = self.active_page().await?;
        self.network_monitoring_enabled
            .store(true, Ordering::SeqCst);
        let page_id = page.target_id().as_ref().to_string();
        if !self.network_monitor_pages.lock().await.insert(page_id) {
            return Ok(());
        }

        // 启用 Network 域
        page.execute(EnableParams::default())
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        // 监听请求事件
        let mut events = page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let network_requests = self.network_requests.clone();
        let capture_sensitive_data = self.config.capture_sensitive_data;

        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let post_data = event
                    .request
                    .post_data_entries
                    .as_ref()
                    .and_then(|entries| {
                        entries
                            .first()
                            .and_then(|e| e.bytes.clone())
                            .map(String::from)
                    });

                let request = crate::types::NetworkRequest {
                    request_id: event.request_id.as_ref().to_string(),
                    url: event.request.url.clone(),
                    method: event.request.method.clone(),
                    resource_type: event
                        .r#type
                        .as_ref()
                        .map(|t| format!("{:?}", t))
                        .unwrap_or_default(),
                    headers: sanitize_headers(
                        serde_json::to_value(&event.request.headers)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                        capture_sensitive_data,
                    ),
                    post_data: capture_sensitive_data.then_some(post_data).flatten(),
                };

                let mut reqs = network_requests.lock().await;
                reqs.push(request);
                // 限制最大数量，防止内存溢出
                if reqs.len() > 1000 {
                    reqs.remove(0);
                }
            }
            debug!("Network monitoring stream ended");
        });

        info!("Network monitoring enabled");
        Ok(())
    }

    /// 获取收集的网络请求
    pub async fn get_network_requests(&self) -> Result<Vec<crate::types::NetworkRequest>> {
        let requests = self.network_requests.lock().await.clone();
        Ok(requests)
    }

    /// 清除收集的网络请求
    pub async fn clear_network_requests(&self) -> Result<()> {
        self.network_requests.lock().await.clear();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 控制台监控
// ---------------------------------------------------------------------------

impl BrowserEngine {
    /// 启用控制台监控
    ///
    /// 启用 CDP Runtime 域，捕获所有 console.log/warn/error/info 调用。
    /// 收集的消息可通过 `get_console_messages()` 获取。
    pub async fn enable_console_monitoring(&self) -> Result<()> {
        use chromiumoxide::cdp::js_protocol::runtime::{EnableParams, EventConsoleApiCalled};
        use futures::StreamExt;

        let page = self.active_page().await?;
        self.console_monitoring_enabled
            .store(true, Ordering::SeqCst);
        let page_id = page.target_id().as_ref().to_string();
        if !self.console_monitor_pages.lock().await.insert(page_id) {
            return Ok(());
        }

        // 启用 Runtime 域
        page.execute(EnableParams::default())
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        // 监听 console API 调用
        let mut events = page
            .event_listener::<EventConsoleApiCalled>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let console_messages = self.console_messages.clone();

        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                // 解析消息文本
                let text = event
                    .args
                    .iter()
                    .filter_map(|arg| {
                        // 尝试从 RemoteObject 获取值
                        if let Some(ref value) = arg.value {
                            match value {
                                serde_json::Value::String(s) => Some(s.clone()),
                                serde_json::Value::Number(n) => Some(n.to_string()),
                                serde_json::Value::Bool(b) => Some(b.to_string()),
                                serde_json::Value::Null => Some("null".to_string()),
                                _ => None,
                            }
                        } else if arg.description.is_some() {
                            Some(arg.description.clone().unwrap_or_default())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                // 处理 stack_trace (Arc<StackTrace>)
                let (url, line_number) = event
                    .stack_trace
                    .as_ref()
                    .and_then(|st| st.call_frames.first())
                    .map(|f| (Some(f.url.clone()), Some(f.line_number)))
                    .unwrap_or((None, None));

                let message = crate::types::ConsoleMessage {
                    level: format!("{:?}", event.r#type),
                    text,
                    url,
                    line_number,
                    timestamp: *event.timestamp.inner(),
                };

                let mut msgs = console_messages.lock().await;
                msgs.push(message);
                // 限制最大数量，防止内存溢出
                if msgs.len() > 1000 {
                    msgs.remove(0);
                }
            }
            debug!("Console monitoring stream ended");
        });

        info!("Console monitoring enabled");
        Ok(())
    }

    /// 获取收集的控制台消息
    pub async fn get_console_messages(&self) -> Result<Vec<crate::types::ConsoleMessage>> {
        let messages = self.console_messages.lock().await.clone();
        Ok(messages)
    }

    /// 清除收集的控制台消息
    pub async fn clear_console_messages(&self) -> Result<()> {
        self.console_messages.lock().await.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::SnapshotNode;

    fn node(ref_id: &str, children: Vec<SnapshotNode>) -> SnapshotNode {
        SnapshotNode {
            ref_id: ref_id.to_string(),
            role: "generic".to_string(),
            name: String::new(),
            value: None,
            description: None,
            bounds: None,
            attributes: HashMap::new(),
            children,
        }
    }

    #[test]
    fn collect_snapshot_nodes_is_recursive() {
        let nodes = vec![node(
            "root",
            vec![node("child", vec![node("leaf", vec![])])],
        )];
        let mut collected = Vec::new();
        collect_snapshot_nodes(&nodes, &mut collected);
        let refs: Vec<&str> = collected.iter().map(|node| node.ref_id.as_str()).collect();
        assert_eq!(refs, vec!["root", "child", "leaf"]);
    }

    #[test]
    fn file_paths_are_restricted_to_allowed_roots() {
        let root = std::env::temp_dir().join(format!("agent-browser-{}", uuid::Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("agent-browser-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create allowed root");
        std::fs::create_dir_all(&outside).expect("create outside root");
        let allowed_file = root.join("upload.txt");
        let outside_file = outside.join("secret.txt");
        std::fs::write(&allowed_file, "allowed").expect("write allowed file");
        std::fs::write(&outside_file, "outside").expect("write outside file");

        assert!(
            validate_file_path(allowed_file.to_str().unwrap(), std::slice::from_ref(&root)).is_ok()
        );
        assert!(
            validate_file_path(outside_file.to_str().unwrap(), std::slice::from_ref(&root))
                .is_err()
        );
        assert!(
            validate_directory_path(
                root.join("downloads/new").to_str().unwrap(),
                std::slice::from_ref(&root)
            )
            .is_ok()
        );

        std::fs::remove_dir_all(root).expect("remove allowed root");
        std::fs::remove_dir_all(outside).expect("remove outside root");
    }

    #[test]
    fn wildcard_origin_matches_scheme_and_port() {
        let https = Url::parse("https://app.example.com/path").unwrap();
        let http = Url::parse("http://app.example.com/path").unwrap();
        let custom_port = Url::parse("https://app.example.com:8443/path").unwrap();

        assert!(origin_matches("https://*.example.com", &https));
        assert!(!origin_matches("https://*.example.com", &http));
        assert!(!origin_matches("https://*.example.com", &custom_port));
        assert!(origin_matches("https://*.example.com:8443", &custom_port));
    }
}

// ---------------------------------------------------------------------------
