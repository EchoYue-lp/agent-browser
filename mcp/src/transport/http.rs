//! Streamable HTTP 传输层实现
//!
//! MCP 2025-11-25 Streamable HTTP 规范
//!
//! ## 特性
//!
//! - 单端点支持 POST 和 GET
//! - 会话管理 (`MCP-Session-Id`)
//! - SSE 流支持
//! - 协议版本头 (`MCP-Protocol-Version`)

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{Mutex, broadcast, oneshot};
use tracing::{debug, error, info, warn};

use super::Transport;
use crate::protocol::{JsonRpcNotification, JsonRpcResponse, MCP_PROTOCOL_VERSION};

/// HTTP 传输层配置
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// MCP 端点 URL
    pub endpoint: String,
    /// 请求头
    pub headers: HashMap<String, String>,
    /// 请求超时（毫秒）
    pub timeout_ms: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            headers: HashMap::new(),
            timeout_ms: 30000,
        }
    }
}

/// Streamable HTTP 客户端传输层
pub struct HttpClientTransport {
    client: reqwest::Client,
    config: HttpConfig,
    next_id: Arc<AtomicU64>,
    /// 会话 ID
    session_id: Arc<Mutex<Option<String>>>,
    /// 等待响应的请求映射（用于 SSE 模式）
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    /// 通知广播通道
    notification_tx: broadcast::Sender<JsonRpcNotification>,
    /// SSE 监听任务
    _sse_task: Option<tokio::task::JoinHandle<()>>,
}

impl HttpClientTransport {
    /// 创建新的 HTTP 客户端传输层
    pub fn new(config: HttpConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client");

        let next_id = Arc::new(AtomicU64::new(1));
        let session_id = Arc::new(Mutex::new(None));
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (notification_tx, _) = broadcast::channel(64);

        Self {
            client,
            config,
            next_id,
            session_id,
            pending,
            notification_tx,
            _sse_task: None,
        }
    }

    /// 启动 SSE 监听（可选）
    pub async fn start_sse_listener(&mut self) -> anyhow::Result<()> {
        let client = self.client.clone();
        let endpoint = self.config.endpoint.clone();
        let headers = self.config.headers.clone();
        let session_id = self.session_id.clone();
        let pending = self.pending.clone();
        let notification_tx = self.notification_tx.clone();

        let task = tokio::spawn(async move {
            Self::run_sse_listener(
                client,
                &endpoint,
                &headers,
                session_id,
                pending,
                notification_tx,
            )
            .await;
        });

        self._sse_task = Some(task);
        Ok(())
    }

    async fn run_sse_listener(
        client: reqwest::Client,
        endpoint: &str,
        headers: &HashMap<String, String>,
        session_id: Arc<Mutex<Option<String>>>,
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        notification_tx: broadcast::Sender<JsonRpcNotification>,
    ) {
        let mut retry_ms: u64 = 2000;

        loop {
            debug!("HTTP SSE: Connecting to {}", endpoint);

            let mut builder = client
                .get(endpoint)
                .header("Accept", "text/event-stream")
                .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION);

            // 添加会话 ID
            {
                let sid = session_id.lock().await;
                if let Some(ref id) = *sid {
                    builder = builder.header("MCP-Session-Id", id);
                }
            }

            for (k, v) in headers {
                builder = builder.header(k, v);
            }

            match builder.send().await {
                Ok(response) => {
                    if response.status() == 405 {
                        debug!("HTTP SSE: Server does not support GET for SSE");
                        return;
                    }

                    if !response.status().is_success() {
                        warn!(
                            "HTTP SSE: Connection failed with status {}",
                            response.status()
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                        continue;
                    }

                    info!("HTTP SSE: Connected");

                    let mut stream = response.bytes_stream();
                    let mut buffer = String::new();

                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(bytes) => {
                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                    buffer.push_str(text);

                                    while let Some(pos) = buffer.find("\n\n") {
                                        let event_block = buffer[..pos].to_string();
                                        buffer = buffer[pos + 2..].to_string();

                                        if let Some(data) = Self::parse_sse_event(&event_block) {
                                            Self::handle_sse_message(
                                                &data,
                                                &pending,
                                                &notification_tx,
                                            )
                                            .await;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                error!("HTTP SSE: Stream error: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("HTTP SSE: Connection error: {}", e);
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
        }
    }

    fn parse_sse_event(event_block: &str) -> Option<String> {
        let mut data_lines = Vec::new();

        for line in event_block.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                data_lines.push(data.trim());
            }
        }

        if data_lines.is_empty() {
            None
        } else {
            Some(data_lines.join("\n"))
        }
    }

    async fn handle_sse_message(
        data: &str,
        pending: &Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        notification_tx: &broadcast::Sender<JsonRpcNotification>,
    ) {
        debug!("HTTP SSE: Received: {}", data);

        let json: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return,
        };

        if json.get("method").is_some() && json.get("id").is_none() {
            // 通知
            if let Ok(notification) = serde_json::from_value::<JsonRpcNotification>(json) {
                let _ = notification_tx.send(notification);
            }
        }
    }

    /// 获取会话 ID
    pub async fn session_id(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }

    /// 设置会话 ID
    pub async fn set_session_id(&self, id: String) {
        *self.session_id.lock().await = Some(id);
    }

    /// 获取通知接收器
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }
}

#[async_trait]
impl Transport for HttpClientTransport {
    async fn send(
        &self,
        _id: Option<Value>,
        method: &str,
        params: Option<Value>,
    ) -> anyhow::Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let id_str = id.to_string();

        // 构建请求
        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let mut builder = self
            .client
            .post(&self.config.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
            .json(&request_body);

        // 添加会话 ID
        {
            let sid = self.session_id.lock().await;
            if let Some(ref id) = *sid {
                builder = builder.header("MCP-Session-Id", id);
            }
        }

        for (k, v) in &self.config.headers {
            builder = builder.header(k, v);
        }

        let response = builder
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("HTTP request failed: {}", e))?;

        // 提取会话 ID（如果有）
        if let Some(session_id) = response.headers().get("MCP-Session-Id") {
            if let Ok(id) = session_id.to_str() {
                self.set_session_id(id.to_string()).await;
            }
        }

        // 检查 Content-Type
        let content_type = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if content_type.contains("text/event-stream") {
            // SSE 响应
            let (tx, rx) = oneshot::channel();
            {
                let mut pending = self.pending.lock().await;
                pending.insert(id_str.clone(), tx);
            }

            // 等待 SSE 事件
            let timeout_ms = self.config.timeout_ms;
            let response = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx)
                .await
                .map_err(|_| anyhow::anyhow!("SSE response timeout"))?
                .map_err(|_| anyhow::anyhow!("Response channel closed"))?;

            Ok(response)
        } else {
            // JSON 响应
            let response: JsonRpcResponse = response
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to parse JSON response: {}", e))?;

            Ok(response)
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> anyhow::Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let mut builder = self
            .client
            .post(&self.config.endpoint)
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
            .json(&notification);

        {
            let sid = self.session_id.lock().await;
            if let Some(ref id) = *sid {
                builder = builder.header("MCP-Session-Id", id);
            }
        }

        for (k, v) in &self.config.headers {
            builder = builder.header(k, v);
        }

        let _ = builder.send().await;
        Ok(())
    }

    async fn close(&self) -> anyhow::Result<()> {
        // 发送 DELETE 请求终止会话
        if let Some(session_id) = self.session_id().await {
            let builder = self
                .client
                .delete(&self.config.endpoint)
                .header("MCP-Session-Id", session_id);

            let _ = builder.send().await;
        }

        Ok(())
    }
}
