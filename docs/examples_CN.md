# 使用示例

展示 Agent Browser 功能的实际示例。

## 目录

- [网页数据抓取](#网页数据抓取)
- [表单填写](#表单填写)
- [CSS 选择器操作](#css-选择器操作)
- [动态内容处理](#动态内容处理)
- [身份认证](#身份认证)
- [文件操作](#文件操作)
- [多标签页操作](#多标签页操作)
- [网络与控制台监控](#网络与控制台监控)
- [视口设置](#视口设置)

## 网页数据抓取

### 基本抓取

```bash
# 导航到页面
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://news.ycombinator.com"}'

# 获取快照了解页面结构
curl http://localhost:3000/snapshot | jq '.data.nodes[] | select(.role == "link") | {name, ref_id}' | head -20

# 使用 JavaScript 提取数据
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "Array.from(document.querySelectorAll(\".titleline > a\")).map(a => ({title: a.textContent, href: a.href})).slice(0, 10)"}'
```

### 提取 PDF 链接

```bash
# 导航到包含 PDF 链接的页面
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/documents"}'

# 查找所有 PDF 链接
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "Array.from(document.querySelectorAll(\"a[href$=\\\".pdf\\\"]\")).map(a => ({text: a.textContent.trim(), href: a.href}))"}'
```

### Rust 示例

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

    // 获取快照
    let snapshot = engine.snapshot().await?;
    println!("找到 {} 个节点", snapshot.nodes.len());

    // 使用 JavaScript 提取链接
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

## 表单填写

### 登录表单

```bash
# 导航到登录页面
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/login"}'

# 填写用户名
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"username\"]", "text": "myuser", "clear_first": true}'

# 填写密码
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"password\"]", "text": "mypassword", "clear_first": true}'

# 提交表单
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "button[type=\"submit\"]"}'

# 等待导航完成
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"selector": ".dashboard", "timeout_ms": 5000}'
```

### 带下拉框的复杂表单

```bash
# 填写文本字段
curl -X POST http://localhost:3000/type-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "#name", "text": "张三"}'

# 按值选择下拉选项
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector": "#country", "value": "cn"}'

# 按文本选择下拉选项
curl -X POST http://localhost:3000/select-option \
  -H "Content-Type: application/json" \
  -d '{"selector": "#city", "value": "北京", "by_text": true}'

# 勾选复选框（点击它）
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[name=\"agree\"]"}'
```

## CSS 选择器操作

### 直接元素访问

```bash
# 通过选择器点击
curl -X POST http://localhost:3000/click-selector \
  -H "Content-Type: application/json" \
  -d '{"selector": "button.primary"}'

# 获取文本内容
curl -X POST http://localhost:3000/get-text \
  -H "Content-Type: application/json" \
  -d '{"selector": ".article-title"}'

# 获取属性
curl -X POST http://localhost:3000/get-attribute \
  -H "Content-Type: application/json" \
  -d '{"selector": "a.download", "attribute": "href"}'

# 检查存在
curl -X POST http://localhost:3000/element-exists \
  -H "Content-Type: application/json" \
  -d '{"selector": ".error-message"}'

# 鼠标悬停
curl -X POST http://localhost:3000/hover \
  -H "Content-Type: application/json" \
  -d '{"selector": ".menu-trigger"}'
```

### 处理 Vue.js/React 组件

```bash
# 许多 SPA 框架使用自定义组件
# 通过底层结构访问它们

# 点击 Vue 组件按钮
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "Array.from(document.querySelectorAll(\".el-menu-item\")).find(el => el.textContent.includes(\"设置\")).click()"}'

# 或通过文本内容查找
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "Array.from(document.querySelectorAll(\"button\")).find(b => b.textContent.includes(\"提交\")).click()"}'
```

## 动态内容处理

### 等待元素

```bash
# 等待选择器出现
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"selector": ".loaded-content", "timeout_ms": 10000}'

# 等待网络空闲
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"idle_duration_ms": 1000, "timeout_ms": 30000}'

# 简单等待
curl -X POST http://localhost:3000/wait \
  -H "Content-Type: application/json" \
  -d '{"timeout_ms": 2000}'
```

### 滚动加载更多

```bash
# 向下滚动
curl -X POST http://localhost:3000/act \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax1", "action": "scroll", "direction": "down", "amount": 500}'

# 或使用 JavaScript 进行无限滚动
curl -X POST http://localhost:3000/evaluate \
  -H "Content-Type: application/json" \
  -d '{"script": "window.scrollTo(0, document.body.scrollHeight)"}'
```

## 身份认证

### 基于 Cookie 的认证

```bash
# 设置认证 Cookie
curl -X POST http://localhost:3000/cookies \
  -H "Content-Type: application/json" \
  -d '{
    "cookies": [
      {"name": "session_id", "value": "abc123", "domain": "example.com"}
    ]
  }'

# 导航到受保护页面
curl -X POST http://localhost:3000/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/dashboard"}'
```

### 持久化会话

```bash
# 使用 profile 目录持久化 Cookie
BROWSER_PROFILE_DIR=/path/to/profile ./target/release/agent-browser-http
```

或在 Rust 中：

```rust
let config = BrowserConfig::default()
    .with_profile_dir("/path/to/profile");
```

## 文件操作

### 下载文件

```bash
# 通过 URL 下载
curl -X POST http://localhost:3000/download \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.pdf", "save_path": "/downloads"}'

# 点击并下载
curl -X POST http://localhost:3000/click-download \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax10", "save_path": "/downloads", "timeout_ms": 60000}'
```

### 文件上传

```bash
# 上传到文件输入框
curl -X POST http://localhost:3000/upload \
  -H "Content-Type: application/json" \
  -d '{"ref_id": "ax5", "file_path": "/path/to/document.pdf"}'
```

## 多标签页操作

### 操作多个标签页

```bash
# 列出所有标签页
curl http://localhost:3000/tabs

# 切换到指定标签页
curl -X POST http://localhost:3000/tabs/TAB_ID/activate

# 关闭标签页
curl -X DELETE http://localhost:3000/tabs/TAB_ID
```

### Rust 示例

```rust
// 列出并切换标签页
let tabs = engine.list_tabs().await?;
for tab in &tabs {
    println!("标签页: {} - {}", tab.tab_id, tab.title);
}

// 切换到第一个标签页
if !tabs.is_empty() {
    engine.activate_tab(&tabs[0].tab_id).await?;
}

// 关闭其他标签页
for tab in tabs.iter().skip(1) {
    engine.close_tab(&tab.tab_id).await?;
}
```

## 网络与控制台监控

### 网络请求监控

```bash
# 启用网络监控
curl -X POST http://localhost:3000/network/enable

# 获取捕获的网络请求
curl http://localhost:3000/network/requests

# 清除请求记录
curl -X POST http://localhost:3000/network/clear
```

### 控制台消息监控

```bash
# 启用控制台监控
curl -X POST http://localhost:3000/console/enable

# 获取控制台消息
curl http://localhost:3000/console/messages

# 清除消息记录
curl -X POST http://localhost:3000/console/clear
```

## 视口设置

### 模拟不同设备

```bash
# 设置视口大小（模拟桌面）
curl -X POST http://localhost:3000/viewport \
  -H "Content-Type: application/json" \
  -d '{"width": 1920, "height": 1080}'

# 设置视口大小（模拟移动设备）
curl -X POST http://localhost:3000/viewport \
  -H "Content-Type: application/json" \
  -d '{"width": 375, "height": 667, "device_scale_factor": 2}'

# 获取当前视口大小
curl http://localhost:3000/viewport
```

## 截图

### 各种截图选项

```bash
# 视口截图
curl http://localhost:3000/screenshot | jq -r '.data.image' | base64 -d > viewport.png

# 全页面截图
curl "http://localhost:3000/screenshot?full_page=true" | jq -r '.data.image' | base64 -d > fullpage.png

# 元素截图
curl "http://localhost:3000/screenshot?selector=.main-content" | jq -r '.data.image' | base64 -d > element.png
```

### Rust 示例

```rust
// 截图
let screenshot = engine.screenshot(Some(ScreenshotOptions {
    full_page: Some(true),
    selector: None,
})).await?;

std::fs::write("screenshot.png", base64::decode(&screenshot.data)?)?;
```

## 键盘快捷键

### 发送快捷键

```bash
# 复制
curl -X POST http://localhost:3000/shortcut \
  -H "Content-Type: application/json" \
  -d '{"shortcut": "copy"}'

# 粘贴
curl -X POST http://localhost:3000/shortcut \
  -H "Content-Type: application/json" \
  -d '{"shortcut": "paste"}'

# 全选
curl -X POST http://localhost:3000/shortcut \
  -H "Content-Type: application/json" \
  -d '{"shortcut": "selectAll"}'

# 刷新
curl -X POST http://localhost:3000/shortcut \
  -H "Content-Type: application/json" \
  -d '{"shortcut": "refresh"}'
```

### 带修饰键的按键

```bash
# Ctrl+C
curl -X POST http://localhost:3000/press-key \
  -H "Content-Type: application/json" \
  -d '{"key": "c", "modifiers": ["control"]}'

# Ctrl+Shift+I（开发者工具）
curl -X POST http://localhost:3000/press-key \
  -H "Content-Type: application/json" \
  -d '{"key": "i", "modifiers": ["control", "shift"]}'
```

## 错误处理

### 健壮的抓取

```rust
use agent_browser_core::{BrowserEngine, BrowserConfig, Error};

async fn robust_click(engine: &BrowserEngine, selector: &str) -> Result<(), Error> {
    // 先等待元素
    match engine.wait_for_selector(selector, 5000).await {
        Ok(_) => {},
        Err(_) => {
            // 如果没找到，尝试滚动
            engine.evaluate("window.scrollBy(0, 500)").await?;
            engine.wait_for_selector(selector, 5000).await?;
        }
    }

    // 带重试的点击
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