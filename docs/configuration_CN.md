# 配置指南

Agent Browser 的详细配置选项。

## BrowserConfig

浏览器设置的主要配置结构体。

### 字段

| 字段 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `headless` | `HeadlessMode` | `New` | 无头浏览器模式 |
| `browser_path` | `Option<PathBuf>` | 自动检测 | Chrome/Chromium 可执行文件路径 |
| `profile_dir` | `Option<PathBuf>` | `None` | 用户数据目录，用于持久化 Cookie |
| `navigation_timeout_ms` | `u64` | `30000` | 导航超时时间（毫秒） |
| `action_timeout_ms` | `u64` | `10000` | 操作超时时间（毫秒） |
| `stealth` | `bool` | `true` | 启用反检测脚本 |
| `no_sandbox` | `bool` | `false` | 显式添加 Chrome `--no-sandbox` 参数 |
| `extra_args` | `Vec<String>` | `[]` | 额外的 Chrome 启动参数 |
| `allowed_file_roots` | `Vec<PathBuf>` | 当前目录和临时目录 | 上传/下载文件系统边界 |
| `allowed_origins` | `Vec<String>` | `[]` | 可选的精确/通配 Origin 白名单 |
| `blocked_origins` | `Vec<String>` | `[]` | 优先匹配的 Origin 黑名单 |
| `allow_private_networks` | `bool` | `false` | 是否允许回环、私网和链路本地地址 |
| `capture_sensitive_data` | `bool` | `false` | 网络监控中是否保留认证头和请求体 |

### HeadlessMode

```rust
pub enum HeadlessMode {
    /// 显示浏览器窗口
    None,
    /// 旧版无头模式（可被检测）
    Old,
    /// 新版无头模式（Chrome 112+，更难检测）
    New,
}
```

## 构建器方法

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

let config = BrowserConfig::default()
    // 无头模式
    .with_headless(HeadlessMode::New)

    // 自定义浏览器路径
    .with_browser_path("/usr/bin/google-chrome")

    // 持久化 Cookie 和会话
    .with_profile_dir("~/.config/chrome-profile")

    // 启用/禁用反检测
    .with_stealth(true)

    // 除非运行环境无法支持，否则保持 Chrome 沙箱开启
    .with_no_sandbox(false)

    // 限制浏览器网络和文件系统访问
    .with_allowed_origins(["https://example.com", "https://*.example.org"])
    .with_blocked_origins(["https://admin.example.com"])
    .with_private_networks(false)
    .with_allowed_file_roots(["/tmp/agent-browser"])

    // 添加 Chrome 参数
    .with_arg("--disable-web-security")
    .with_arg("--window-size=1920,1080");
```

## 预设配置

### 有头模式（显示浏览器）

```rust
let config = BrowserConfig::headed();
```

### 无头模式（新版）

```rust
let config = BrowserConfig::headless();
```

### 无头模式（旧版）

```rust
let config = BrowserConfig::headless_old();
```

## 环境变量

### HTTP Server

| 变量 | 描述 | 默认值 |
|------|------|--------|
| `BROWSER_HTTP_HOST` | 监听地址；非回环地址必须配置 API Key | `127.0.0.1` |
| `BROWSER_HTTP_PORT` | 服务器端口 | `3000` |
| `BROWSER_HEADLESS` | `false` 为有头，`old` 为旧无头，其他值为新无头 | 新无头 |
| `BROWSER_STEALTH` | 是否启用 stealth 脚本 | `true` |
| `BROWSER_NO_SANDBOX` | 显式禁用 Chrome 沙箱 | `false` |
| `BROWSER_API_KEY` | API 认证密钥 | - |
| `BROWSER_DEFAULT_TIMEOUT_MS` | 默认超时时间（毫秒） | `30000` |
| `BROWSER_ALLOWED_FILE_ROOTS` | 按平台路径分隔符连接的上传/下载允许目录 | 当前目录和临时目录 |
| `BROWSER_ALLOWED_ORIGINS` | 逗号分隔的精确/通配 Origin 白名单 | 不限制公网 Origin |
| `BROWSER_BLOCKED_ORIGINS` | 逗号分隔的 Origin 黑名单 | - |
| `BROWSER_ALLOW_PRIVATE_NETWORKS` | 允许回环、私网和链路本地目标 | `false` |
| `BROWSER_CAPTURE_SENSITIVE_DATA` | 在网络日志中保留凭证和请求体 | `false` |
| `BROWSER_NAVIGATION_TIMEOUT_MS` | 导航超时 | `30000` |
| `BROWSER_ACTION_TIMEOUT_MS` | 可操作性等待超时 | `10000` |
| `BROWSER_MCP_CAPS` | MCP 能力白名单（`network,storage,files,devtools`） | 全部 |

### 示例

```bash
# 使用自定义设置启动 HTTP 服务器
BROWSER_HTTP_PORT=8080 \
BROWSER_HEADLESS=1 \
BROWSER_NO_SANDBOX=false \
BROWSER_API_KEY=secret123 \
BROWSER_ALLOWED_FILE_ROOTS=/tmp:/path/to/workspace \
BROWSER_ALLOWED_ORIGINS=https://example.com,https://*.example.org \
BROWSER_DEFAULT_TIMEOUT_MS=60000 \
./target/release/agent-browser-http
```

## Chrome 启动参数

### 常用参数

| 参数 | 描述 |
|------|------|
| `--disable-web-security` | 禁用同源策略 |
| `--disable-features=IsolateOrigins,site-per-process` | 禁用站点隔离 |
| `--window-size=WIDTH,HEIGHT` | 设置窗口大小 |
| `--disable-gpu` | 禁用 GPU 硬件加速 |
| `--no-sandbox` | 禁用沙箱；仅在运行环境无法支持 Chrome 沙箱时使用 |
| `--disable-setuid-sandbox` | 禁用 setuid 沙箱 |
| `--disable-dev-shm-usage` | 使用 /tmp 替代 /dev/shm |
| `--disable-blink-features=AutomationControlled` | 隐藏自动化指示器 |

### 添加参数

```rust
let config = BrowserConfig::default()
    .with_arg("--disable-web-security")
    .with_arg("--window-size=1920,1080");
```

## 反检测配置

### Stealth 模式

当 `stealth: true` 时，Agent Browser 会注入 JavaScript 来：

1. 隐藏 `navigator.webdriver` 属性
2. 修改 `navigator.plugins` 使其看起来正常
3. 覆盖 `navigator.languages`
4. 隐藏 Chrome 自动化指示器

### 新版无头模式

Chrome 112+ 引入了新版无头模式：

- 与有头模式共享相同的浏览器代码
- 比旧版无头模式更难检测
- 推荐用于生产环境爬取

```rust
let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_stealth(true);
```

## Cookie 持久化

使用 `profile_dir` 持久化 Cookie 和会话数据：

```rust
let config = BrowserConfig::default()
    .with_profile_dir("/path/to/profile");
```

这可以实现：
- 跨会话保持登录状态
- 持久化 Cookie
- 保留本地存储数据

## 日志配置

### 启用调试日志

```bash
# HTTP 服务器
RUST_LOG=agent_browser_http=debug,agent_browser_core=debug \
./target/release/agent-browser-http

# MCP 服务器
RUST_LOG=agent_browser_mcp=debug,agent_browser_core=debug \
./target/release/agent-browser-mcp
```

### 日志级别

| 级别 | 描述 |
|------|------|
| `error` | 仅错误 |
| `warn` | 警告和错误 |
| `info` | 一般信息（默认） |
| `debug` | 详细调试信息 |
| `trace` | 非常详细的输出 |

### MCP 日志

MCP 服务端支持通过协议配置日志级别：

```json
{
  "method": "logging/setLevel",
  "params": {
    "level": "debug"
  }
}
```

支持级别: `debug`, `info`, `notice`, `warning`, `error`, `critical`, `alert`, `emergency`

## 平台特定说明

### macOS

Chrome 通常安装在：
```
/Applications/Google Chrome.app/Contents/MacOS/Google Chrome
```

支持自动检测。

### Linux

检查的 Chrome 路径：
```
/usr/bin/google-chrome
/usr/bin/chromium
/usr/bin/chromium-browser
```

### Windows

检查的 Chrome 路径：
```
C:\Program Files\Google\Chrome\Application\chrome.exe
C:\Program Files (x86)\Google\Chrome\Application\chrome.exe
```

## 示例

### 生产环境爬取

```rust
use agent_browser_core::{BrowserConfig, HeadlessMode};

let config = BrowserConfig::default()
    .with_headless(HeadlessMode::New)
    .with_stealth(true)
    .with_arg("--disable-blink-features=AutomationControlled")
    .with_arg("--window-size=1920,1080");
```

### 开发/调试

```rust
let config = BrowserConfig::headed()
    .with_stealth(false);
```

### Docker 环境

```rust
let config = BrowserConfig::headless()
    .with_stealth(true)
    .with_no_sandbox(true)
    .with_arg("--disable-setuid-sandbox")
    .with_arg("--disable-dev-shm-usage");
```

`with_no_sandbox(true)` 会削弱进程隔离，应优先使用支持 Chrome 沙箱的容器或运行环境。
