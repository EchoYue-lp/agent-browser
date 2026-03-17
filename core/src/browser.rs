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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, info, warn};

use crate::actions::{ActionKind, ActionResult};
use crate::error::{Error, Result};
use crate::snapshot;
use crate::snapshot::PageSnapshot;
use crate::types::{
    BrowserConfig, CookieInfo, DownloadOptions, DownloadResult, DownloadStatus, HeadlessMode,
    KeyModifier, ScreenshotOptions, SetCookieParam, TabInfo,
};

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
/// use agent_browser_core::{BrowserEngine, BrowserConfig};
///
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// let engine = BrowserEngine::new(BrowserConfig::headed());
/// engine.navigate("https://example.com").await?;
/// let snapshot = engine.snapshot().await?;
/// engine.click("ax1").await?;
/// engine.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct BrowserEngine {
    /// Browser instance.
    browser: Mutex<Option<Arc<Browser>>>,
    /// Active page.
    active_page: Mutex<Option<Page>>,
    /// Tab mapping (tab_id -> Page).
    tabs: Mutex<HashMap<String, Page>>,
    /// Active tab_id.
    active_tab_id: Mutex<Option<String>>,
    /// Configuration.
    config: BrowserConfig,
    /// iframe context stack (supports nested iframes).
    iframe_stack: Mutex<Vec<IframeContext>>,
    /// ref_id -> frame_id mapping (updated on each snapshot).
    iframe_mapping: Mutex<HashMap<String, String>>,
    /// frame_id -> execution_context_id mapping.
    execution_contexts: Mutex<HashMap<String, i64>>,
    /// Currently active frame_id (None means main frame).
    active_frame_id: Mutex<Option<String>>,
    /// Currently active execution context ID.
    active_context_id: Mutex<Option<i64>>,
    /// Download directory.
    download_dir: Mutex<Option<PathBuf>>,
    /// Download event broadcaster.
    download_events: broadcast::Sender<DownloadResult>,
}

impl BrowserEngine {
    /// Create a new browser engine (not launched).
    pub fn new(config: BrowserConfig) -> Self {
        let (download_events, _) = broadcast::channel(16);
        Self {
            browser: Mutex::new(None),
            active_page: Mutex::new(None),
            tabs: Mutex::new(HashMap::new()),
            active_tab_id: Mutex::new(None),
            config,
            iframe_stack: Mutex::new(Vec::new()),
            iframe_mapping: Mutex::new(HashMap::new()),
            execution_contexts: Mutex::new(HashMap::new()),
            active_frame_id: Mutex::new(None),
            active_context_id: Mutex::new(None),
            download_dir: Mutex::new(None),
            download_events,
        }
    }

    /// Launch the browser.
    pub async fn launch(&self) -> Result<BrowserHandle> {
        let headless_str = match self.config.headless {
            HeadlessMode::None => "headed",
            HeadlessMode::Old => "headless(old)",
            HeadlessMode::New => "headless(new)",
        };
        info!(
            "Launching browser (mode={}, stealth={})",
            headless_str, self.config.stealth
        );

        let mut builder = ChromeConfig::builder();

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
                .arg("--no-sandbox")
                .arg("--disable-gpu")
                .arg("--window-size=1920,1080");
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
        tokio::spawn(async move {
            use futures::StreamExt;
            while let Some(ev) = handler.next().await {
                debug!("Browser event: {:?}", ev);
            }
            warn!("Browser handler stream ended");
        });

        let arc = Arc::new(browser);
        *self.browser.lock().await = Some(arc.clone());

        info!("Browser launched");
        Ok(BrowserHandle(arc))
    }

    /// 确保浏览器已启动
    async fn ensure_launched(&self) -> Result<BrowserHandle> {
        let guard = self.browser.lock().await;
        if let Some(ref arc) = *guard {
            Ok(BrowserHandle(arc.clone()))
        } else {
            drop(guard);
            self.launch().await
        }
    }

    /// 获取或创建活动页面
    async fn get_or_create_page(&self) -> Result<Page> {
        // 检查现有活动页面
        let mut page_guard = self.active_page.lock().await;
        if let Some(ref page) = *page_guard {
            // 检查页面是否仍然有效
            if page.url().await.is_ok() {
                return Ok(page.clone());
            }
        }

        // 需要创建新页面
        let handle = self.ensure_launched().await?;
        let page = handle
            .new_page("about:blank")
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        *page_guard = Some(page.clone());
        Ok(page)
    }

    /// 获取当前活动页面
    pub async fn active_page(&self) -> Result<Page> {
        let page_guard = self.active_page.lock().await;
        page_guard.clone().ok_or(Error::NoActivePage)
    }

    /// 导航到 URL
    ///
    /// 如果浏览器未启动会自动启动。
    /// 导航成功后更新活动页面。
    pub async fn navigate(&self, url: &str) -> Result<crate::types::NavigateResult> {
        info!("Navigating to: {}", url);

        let page = self.get_or_create_page().await?;

        page.goto(url)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        // 等待页面加载
        tokio::time::timeout(
            std::time::Duration::from_millis(self.config.navigation_timeout_ms),
            page.wait_for_navigation(),
        )
        .await
        .map_err(|_| Error::Timeout("Navigation timeout".into()))?
        .map_err(|e| Error::Cdp(e.to_string()))?;

        let final_url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| url.to_string());

        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        info!("Navigated to: {} (title: {})", final_url, title);

        // 如果启用了反检测，注入脚本
        if self.config.stealth {
            self.inject_stealth_scripts(&page).await?;
        }

        Ok(crate::types::NavigateResult {
            url: url.to_string(),
            title: title.clone(),
            final_url,
        })
    }

    /// 注入反检测脚本
    ///
    /// 隐藏 WebDriver 等自动化特征
    async fn inject_stealth_scripts(&self, page: &Page) -> Result<()> {
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

        page.evaluate(stealth_js)
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        debug!("Stealth scripts injected successfully");
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
        let page = self.active_page().await?;
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        info!("Clicking element by selector: {}", selector);

        // 等待元素出现
        self.wait_for_selector(selector, timeout).await?;

        // 点击元素
        let click_script = format!(
            r#"(function() {{
                const el = document.querySelector('{}');
                if (el) {{
                    el.click();
                    return {{ clicked: true, tagName: el.tagName, text: el.textContent.substring(0, 50) }};
                }}
                return {{ clicked: false, error: 'Element not found' }};
            }})()"#,
            selector.replace('\'', "\\'")
        );

        let result: serde_json::Value = page
            .evaluate(click_script.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        let clicked = result["clicked"].as_bool().unwrap_or(false);
        if clicked {
            let tag = result["tagName"].as_str().unwrap_or("");
            let text = result["text"].as_str().unwrap_or("");
            info!("Clicked <{}>: {}", tag, text);
            Ok(ActionResult {
                success: true,
                message: format!("Clicked <{}>: {}", tag, text),
            })
        } else {
            let error = result["error"].as_str().unwrap_or("Unknown error");
            Err(Error::ElementNotFound(format!(
                "Selector '{}': {}",
                selector, error
            )))
        }
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
        let page = self.active_page().await?;
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        info!("Typing in element by selector: {}", selector);

        // 等待元素出现
        self.wait_for_selector(selector, timeout).await?;

        // 输入文本
        let type_script = format!(
            r#"(function() {{
                const el = document.querySelector('{}');
                if (!el) return {{ success: false, error: 'Element not found' }};

                el.focus();
                if ({}) {{
                    el.value = '';
                }}
                el.value += '{}';

                // 触发事件
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));

                return {{ success: true, value: el.value }};
            }})()"#,
            selector.replace('\'', "\\'"),
            clear_first,
            text.replace('\'', "\\'")
        );

        let result: serde_json::Value = page
            .evaluate(type_script.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        let success = result["success"].as_bool().unwrap_or(false);
        if success {
            let value = result["value"].as_str().unwrap_or("");
            Ok(ActionResult {
                success: true,
                message: format!("Typed text, current value: {}", value),
            })
        } else {
            let error = result["error"].as_str().unwrap_or("Unknown error");
            Err(Error::ElementNotFound(format!(
                "Selector '{}': {}",
                selector, error
            )))
        }
    }

    /// 获取元素的文本内容
    ///
    /// 通过 CSS 选择器获取元素的文本内容。
    pub async fn get_text(&self, selector: &str, timeout_ms: Option<u64>) -> Result<String> {
        let page = self.active_page().await?;
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(selector, timeout).await?;

        let script = format!(
            r#"document.querySelector('{}')?.textContent?.trim() || ''"#,
            selector.replace('\'', "\\'")
        );

        let result: String = page
            .evaluate(script.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

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
        let page = self.active_page().await?;
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(selector, timeout).await?;

        let script = format!(
            r#"document.querySelector('{}')?.getAttribute('{}')"#,
            selector.replace('\'', "\\'"),
            attribute.replace('\'', "\\'")
        );

        let result: Option<String> = page
            .evaluate(script.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        Ok(result)
    }

    /// 检查元素是否存在
    pub async fn element_exists(&self, selector: &str) -> Result<bool> {
        let page = self.active_page().await?;

        let script = format!(
            r#"document.querySelector('{}') !== null"#,
            selector.replace('\'', "\\'")
        );

        let result: bool = page
            .evaluate(script.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

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
        let page = self.active_page().await?;
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(select_selector, timeout).await?;

        // 尝试标准 select 元素
        let script = if by_text {
            format!(
                r#"(function() {{
                    const select = document.querySelector('{}');
                    if (select && select.tagName === 'SELECT') {{
                        for (let opt of select.options) {{
                            if (opt.text === '{}') {{
                                select.value = opt.value;
                                select.dispatchEvent(new Event('change', {{ bubbles: true }}));
                                return {{ success: true, selected: opt.text }};
                            }}
                        }}
                    }}
                    return {{ success: false, error: 'Option not found' }};
                }})()"#,
                select_selector.replace('\'', "\\'"),
                value.replace('\'', "\\'")
            )
        } else {
            format!(
                r#"(function() {{
                    const select = document.querySelector('{}');
                    if (select && select.tagName === 'SELECT') {{
                        select.value = '{}';
                        select.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        return {{ success: true, selected: select.value }};
                    }}
                    return {{ success: false, error: 'Not a select element' }};
                }})()"#,
                select_selector.replace('\'', "\\'"),
                value.replace('\'', "\\'")
            )
        };

        let result: serde_json::Value = page
            .evaluate(script.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

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

            // 查找并点击选项
            let option_selector = if by_text {
                format!(
                    "[data-value='{}'], [title='{}'], :contains('{}')",
                    value, value, value
                )
            } else {
                format!("[data-value='{}'], [value='{}']", value, value)
            };

            self.click_selector(&option_selector, Some(timeout)).await
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
        let page = self.active_page().await?;
        let timeout = timeout_ms.unwrap_or(self.config.action_timeout_ms);

        self.wait_for_selector(selector, timeout).await?;

        let script = format!(
            r#"(function() {{
                const el = document.querySelector('{}');
                if (!el) return {{ success: false, error: 'Element not found' }};

                const rect = el.getBoundingClientRect();
                const x = rect.left + rect.width / 2;
                const y = rect.top + rect.height / 2;

                el.dispatchEvent(new MouseEvent('mouseover', {{
                    bubbles: true,
                    cancelable: true,
                    clientX: x,
                    clientY: y
                }}));
                el.dispatchEvent(new MouseEvent('mouseenter', {{
                    bubbles: true,
                    cancelable: true,
                    clientX: x,
                    clientY: y
                }}));

                return {{ success: true, x: x, y: y }};
            }})()"#,
            selector.replace('\'', "\\'")
        );

        let result: serde_json::Value = page
            .evaluate(script.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        let success = result["success"].as_bool().unwrap_or(false);
        if success {
            Ok(ActionResult {
                success: true,
                message: format!(
                    "Hovered over element at ({}, {})",
                    result["x"].as_f64().unwrap_or(0.0),
                    result["y"].as_f64().unwrap_or(0.0)
                ),
            })
        } else {
            Err(Error::ElementNotFound(format!("Selector '{}'", selector)))
        }
    }

    /// 获取页面快照
    ///
    /// 返回 Accessibility Tree，包含所有可交互元素的 ref_id、role、name。
    /// 如果当前在 iframe 上下文中，将获取该 iframe 内的元素。
    /// 同时更新 iframe 映射表。
    pub async fn snapshot(&self) -> Result<PageSnapshot> {
        let active_frame = self.active_frame_id.lock().await.clone();

        // 检查是否在 iframe 上下文中
        if let Some(frame_id) = active_frame {
            info!("Taking snapshot in iframe context: {}", frame_id);
            return self.snapshot_in_frame().await;
        }

        // 主 frame 上下文
        let page = self.active_page().await?;
        let snapshot = snapshot::generate_snapshot(&page).await?;

        // 更新 iframe 映射
        if !snapshot.iframe_mappings.is_empty() {
            let mut mapping = self.iframe_mapping.lock().await;
            mapping.clear();
            for m in &snapshot.iframe_mappings {
                mapping.insert(m.ref_id.clone(), m.frame_id.clone());
            }
            info!("Updated {} iframe mappings", mapping.len());
        }

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
        crate::actions::dispatch_action(&page, ref_id, action, None).await
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
        let page = self.active_page().await?;

        let result = page
            .evaluate(script)
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        result
            .into_value()
            .map_err(|e| Error::JavaScript(e.to_string()))
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
        let guard = self.browser.lock().await;
        if let Some(ref browser) = *guard {
            browser.pages().await.is_ok()
        } else {
            false
        }
    }

    /// 关闭浏览器
    pub async fn shutdown(&self) -> Result<()> {
        let mut browser_guard = self.browser.lock().await;
        let mut page_guard = self.active_page.lock().await;

        // 清空活动页面
        *page_guard = None;

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
    /// 列出所有标签页
    pub async fn list_tabs(&self) -> Result<Vec<TabInfo>> {
        let tabs = self.tabs.lock().await;
        let active_id = self.active_tab_id.lock().await.clone();

        let mut result = Vec::new();
        for (tab_id, page) in tabs.iter() {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            let title = page.get_title().await.ok().flatten().unwrap_or_default();
            let active = Some(tab_id.as_str()) == active_id.as_deref();

            result.push(TabInfo {
                tab_id: tab_id.clone(),
                url,
                title,
                active,
            });
        }

        Ok(result)
    }

    /// 激活标签页
    pub async fn activate_tab(&self, tab_id: &str) -> Result<()> {
        let tabs = self.tabs.lock().await;

        if !tabs.contains_key(tab_id) {
            return Err(Error::Other(format!("Tab not found: {}", tab_id)));
        }

        let mut active_id = self.active_tab_id.lock().await;
        let mut active_page = self.active_page.lock().await;

        *active_id = Some(tab_id.to_string());
        *active_page = tabs.get(tab_id).cloned();

        info!("Activated tab: {}", tab_id);
        Ok(())
    }

    /// 关闭标签页
    pub async fn close_tab(&self, tab_id: &str) -> Result<()> {
        let mut tabs = self.tabs.lock().await;

        let page = tabs
            .remove(tab_id)
            .ok_or_else(|| Error::Other(format!("Tab not found: {}", tab_id)))?;

        // 关闭页面
        page.close().await.map_err(|e| Error::Cdp(e.to_string()))?;

        // 如果关闭的是活动标签页，切换到下一个
        let mut active_id = self.active_tab_id.lock().await;
        if active_id.as_deref() == Some(tab_id) {
            *active_id = tabs.keys().next().cloned();
            let mut active_page = self.active_page.lock().await;
            *active_page = active_id.as_ref().and_then(|id| tabs.get(id).cloned());
        }

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

            let bounds: Option<serde_json::Value> = page
                .evaluate(js.as_str())
                .await
                .ok()
                .and_then(|v| v.into_value().ok());

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
        let page = self.active_page().await?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        loop {
            let js = format!("!!document.querySelector({:?})", selector);
            let found: bool = page
                .evaluate(js)
                .await
                .ok()
                .and_then(|v| v.into_value().ok())
                .unwrap_or(false);

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
        let page = self.active_page().await?;

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
            let val: serde_json::Value = page
                .evaluate(js)
                .await
                .ok()
                .and_then(|v| v.into_value().ok())
                .unwrap_or_else(|| serde_json::json!({"ready":false,"resources":0}));

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
                warn!("networkidle timeout after {}ms", timeout_ms);
                return Ok(()); // 软超时
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
    pub async fn upload_file(&self, ref_id: &str, file_path: &str) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::dom::{
            GetDocumentParams, QuerySelectorParams, SetFileInputFilesParams,
        };

        let page = self.active_page().await?;
        let selector = format!("[data-agent-ref=\"{}\"]", ref_id);

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
            files: vec![file_path.to_string()],
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

        tokio::spawn(async move {
            if let Some(_event) = events.next().await {
                let _ = page_clone
                    .execute(HandleJavaScriptDialogParams {
                        accept,
                        prompt_text: Some(prompt_text.clone()),
                    })
                    .await;
            }
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

        // 更新活动 frame 和上下文
        {
            let mut active_frame = self.active_frame_id.lock().await;
            *active_frame = Some(frame_id.clone());
        }
        {
            let mut active_ctx = self.active_context_id.lock().await;
            *active_ctx = Some(context_id);
        }

        // 添加到 iframe 栈
        {
            let mut stack = self.iframe_stack.lock().await;
            stack.push(IframeContext {
                frame_id: frame_id.clone(),
                url: None,
            });
            info!(
                "Entered iframe: ref_id={}, frame_id={}, context_id={}, depth={}",
                ref_id,
                frame_id,
                context_id,
                stack.len()
            );
            Ok(stack.len())
        }
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

            // 更新活动 frame 和上下文
            {
                let mut active_frame = self.active_frame_id.lock().await;
                *active_frame = Some(fid.clone());
            }
            {
                let mut active_ctx = self.active_context_id.lock().await;
                *active_ctx = Some(context_id);
            }

            // 添加到 iframe 栈
            let mut stack = self.iframe_stack.lock().await;
            stack.push(IframeContext {
                frame_id: fid.clone(),
                url: None,
            });

            info!(
                "Entered iframe (search): ref_id={}, frame_id={}, depth={}",
                ref_id,
                fid,
                stack.len()
            );

            Ok(stack.len())
        } else {
            Err(Error::ElementNotFound(format!(
                "iframe with ref_id={}",
                ref_id
            )))
        }
    }

    /// 获取指定 frame 的执行上下文 ID
    async fn get_frame_execution_context(&self, frame_id: &str) -> Result<i64> {
        let page = self.active_page().await?;

        // 检查缓存
        {
            let cache = self.execution_contexts.lock().await;
            if let Some(&ctx_id) = cache.get(frame_id) {
                return Ok(ctx_id);
            }
        }

        // 首先尝试使用 page 的 execution_context（适用于主 frame）
        match page.execution_context().await {
            Ok(Some(ctx_id)) => {
                // ExecutionContextId 使用 inner() 方法获取内部的 i64
                let ctx_id = *ctx_id.inner();
                info!("Got execution context {} from page", ctx_id);

                // 缓存
                let mut cache = self.execution_contexts.lock().await;
                cache.insert(frame_id.to_string(), ctx_id);

                return Ok(ctx_id);
            }
            _ => {
                warn!("No execution context available from page");
            }
        }

        // 如果无法获取上下文，返回一个默认值
        warn!("Using default execution context for frame {}", frame_id);
        Ok(0)
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
        let mut stack = self.iframe_stack.lock().await;

        if let Some(ctx) = stack.pop() {
            info!(
                "Exited iframe: frame_id={}, depth={}",
                ctx.frame_id,
                stack.len()
            );
        }

        // 更新活动 frame 和上下文
        {
            let mut active_frame = self.active_frame_id.lock().await;
            *active_frame = stack.last().map(|c| c.frame_id.clone());
        }

        // 如果回到了主 frame，清除上下文；否则获取父 frame 的上下文
        if let Some(parent_ctx) = stack.last() {
            if let Ok(ctx_id) = self.get_frame_execution_context(&parent_ctx.frame_id).await {
                let mut active_ctx = self.active_context_id.lock().await;
                *active_ctx = Some(ctx_id);
            }
        } else {
            let mut active_ctx = self.active_context_id.lock().await;
            *active_ctx = None;
        }

        Ok(stack.len())
    }

    /// 退出所有 iframe
    ///
    /// 清空 iframe 栈，返回到主文档上下文。
    pub async fn exit_all_iframes(&self) -> Result<()> {
        {
            let mut stack = self.iframe_stack.lock().await;
            stack.clear();
            info!("Exited all iframes");
        }

        // 重置活动 frame 和上下文
        {
            let mut active_frame = self.active_frame_id.lock().await;
            *active_frame = None;
        }
        {
            let mut active_ctx = self.active_context_id.lock().await;
            *active_ctx = None;
        }

        Ok(())
    }

    /// 获取当前 iframe 深度
    pub async fn iframe_depth(&self) -> usize {
        self.iframe_stack.lock().await.len()
    }

    /// 在当前上下文中执行 JavaScript
    ///
    /// 如果当前在 iframe 上下文中，将尝试在该 iframe 内执行脚本。
    pub async fn evaluate_in_context(&self, script: &str) -> Result<serde_json::Value> {
        let page = self.active_page().await?;
        let stack = self.iframe_stack.lock().await.clone();

        if let Some(ctx) = stack.last() {
            // 在 iframe 上下文中执行
            info!("Evaluating script in iframe context: {}", ctx.frame_id);

            // 使用 JavaScript 在 iframe 中执行
            // 注意：对于跨域 iframe，这种方法可能受限
            let wrapped_script = format!(
                r#"(function() {{
                    const iframes = document.querySelectorAll('iframe');
                    for (const iframe of iframes) {{
                        try {{
                            if (iframe.name === '{frame_id}' ||
                                iframe.src.includes('{frame_id}')) {{
                                return iframe.contentWindow.eval({script:?});
                            }}
                        }} catch (e) {{
                            // 跨域限制
                        }}
                    }}
                    // 尝试通过 frame name 访问
                    try {{
                        if (window.frames['{frame_id}']) {{
                            return window.frames['{frame_id}'].eval({script:?});
                        }}
                    }} catch (e) {{}}
                    throw new Error('iframe not accessible');
                }})()"#,
                frame_id = ctx.frame_id,
                script = script
            );

            let result = page
                .evaluate(wrapped_script.as_str())
                .await
                .map_err(|e| Error::JavaScript(e.to_string()))?;

            result
                .into_value()
                .map_err(|e| Error::JavaScript(e.to_string()))
        } else {
            // 在主文档上下文中执行
            let result = page
                .evaluate(script)
                .await
                .map_err(|e| Error::JavaScript(e.to_string()))?;

            result
                .into_value()
                .map_err(|e| Error::JavaScript(e.to_string()))
        }
    }

    /// 获取当前 iframe 的快照
    ///
    /// 如果当前在 iframe 上下文中，将获取该 iframe 内的元素快照。
    pub async fn snapshot_in_frame(&self) -> Result<PageSnapshot> {
        use chromiumoxide::cdp::browser_protocol::accessibility::{
            EnableParams, GetFullAxTreeParams,
        };

        let page = self.active_page().await?;
        let active_frame = self.active_frame_id.lock().await.clone();

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
                snapshot_id: uuid::Uuid::new_v4().to_string(),
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
        let (nodes, _) = crate::snapshot::process_ax_nodes_in_frame(&page, &ax_nodes, 0).await?;

        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        Ok(PageSnapshot {
            snapshot_id: uuid::Uuid::new_v4().to_string(),
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
    pub async fn setup_download(&self, save_path: Option<&str>) -> Result<PathBuf> {
        use chromiumoxide::cdp::browser_protocol::browser::SetDownloadBehaviorParams;

        let page = self.active_page().await?;

        // 确定下载目录
        let download_dir = if let Some(path) = save_path {
            PathBuf::from(path)
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
        use chromiumoxide::cdp::browser_protocol::browser::{
            DownloadProgressState, EventDownloadProgress,
        };
        use futures::StreamExt;

        let page = self.active_page().await?;
        let timeout = Duration::from_millis(timeout_ms);
        let start = Instant::now();

        // 监听下载进度事件
        let mut events = page
            .event_listener::<EventDownloadProgress>()
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        loop {
            if start.elapsed() > timeout {
                return Err(Error::Timeout("Download wait timeout".to_string()));
            }

            // 使用 tokio::time::timeout 来处理超时
            match tokio::time::timeout(Duration::from_millis(100), events.next()).await {
                Ok(Some(event)) => {
                    // If guid is specified, check if it matches
                    if let Some(g) = guid
                        && event.guid != g
                    {
                        continue;
                    }

                    let state = match event.state {
                        DownloadProgressState::InProgress => DownloadStatus::InProgress,
                        DownloadProgressState::Completed => DownloadStatus::Completed,
                        DownloadProgressState::Canceled => DownloadStatus::Canceled,
                    };

                    // 检查是否完成
                    match state {
                        DownloadStatus::Completed
                        | DownloadStatus::Canceled
                        | DownloadStatus::Interrupted => {
                            let download_dir = self.download_dir.lock().await.clone();
                            let filename = format!("download_{}", event.guid);
                            let file_path = if let Some(ref dir) = download_dir {
                                dir.join(&filename)
                            } else {
                                PathBuf::from(&filename)
                            };

                            let result = DownloadResult {
                                guid: event.guid.clone(),
                                filename: filename.clone(),
                                file_path: file_path.to_string_lossy().to_string(),
                                size: Some(event.received_bytes as u64),
                                mime_type: None,
                                status: state,
                            };

                            // 广播下载完成事件
                            let _ = self.download_events.send(result.clone());

                            info!("Download completed: {} -> {:?}", event.guid, file_path);
                            return Ok(result);
                        }
                        _ => {
                            // 继续等待
                            debug!(
                                "Download in progress: {} ({:?})",
                                event.guid, event.received_bytes
                            );
                        }
                    }
                }
                Ok(None) => {
                    return Err(Error::Cdp("Download event stream closed".to_string()));
                }
                Err(_) => {
                    // 超时，继续循环检查
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

        // 记录当前 URL
        let page = self.active_page().await?;

        // 触发下载：使用 JavaScript 创建隐藏的下载链接
        let js = format!(
            r#"
            (function() {{
                const a = document.createElement('a');
                a.href = '{}';
                a.download = '';
                a.style.display = 'none';
                document.body.appendChild(a);
                a.click();
                document.body.removeChild(a);
                return true;
            }})()
            "#,
            url
        );

        page.evaluate(js.as_str())
            .await
            .map_err(|e| Error::JavaScript(e.to_string()))?;

        info!("Download triggered: {}", url);

        // 等待下载完成
        self.wait_for_download(None, timeout).await
    }

    /// 点击元素并等待下载
    ///
    /// 点击指定元素后等待下载完成。
    pub async fn click_and_download(
        &self,
        ref_id: &str,
        options: Option<DownloadOptions>,
    ) -> Result<DownloadResult> {
        let opts = options.unwrap_or_default();
        let timeout = opts.timeout_ms.unwrap_or(60000);

        // 设置下载目录
        self.setup_download(opts.save_path.as_deref()).await?;

        // 点击元素
        self.click(ref_id).await?;

        info!("Clicked element {} for download", ref_id);

        // 等待下载完成
        self.wait_for_download(None, timeout).await
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
