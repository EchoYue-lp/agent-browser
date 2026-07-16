//! SSE (Server-Sent Events) 传输层实现
//!
//! MCP 2025-11-25 Streamable HTTP 规范中的 SSE 支持
//!
//! ## 架构
//!
//! ```text
//! Client                          Server
//!   |--- GET /sse (keep-alive) --->|     建立 SSE 连接
//!   |<-- SSE stream (messages) ----|     接收服务端消息
//!   |--- POST / (request) -------->|     发送请求
//!   |<-- 202 Accepted -------------|     确认收到
//!   |<-- SSE: {rpc-response} ------|     响应通过 SSE 推送
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{Mutex, broadcast, oneshot};
use tracing::{debug, error, info, warn};

use super::Transport;
use crate::protocol::{ERR_INTERNAL, JsonRpcNotification, JsonRpcResponse, MCP_PROTOCOL_VERSION};

/// SSE 传输层配置
#[derive(Debug, Clone)]
pub struct SseConfig {
    /// MCP 端点 URL
    pub endpoint: String,
    /// 请求头
    pub headers: HashMap<String, String>,
    /// 请求超时（毫秒）
    pub timeout_ms: u64,
}

impl Default for SseConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            headers: HashMap::new(),
            timeout_ms: 30000,
        }
    }
}

/// SSE 传输层客户端
pub struct SseClientTransport {
    client: reqwest::Client,
    config: SseConfig,
    next_id: Arc<AtomicU64>,
    /// 等待响应的请求映射
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    /// 通知广播通道
    notification_tx: broadcast::Sender<JsonRpcNotification>,
    /// SSE 任务句柄
    _sse_task: Option<tokio::task::JoinHandle<()>>,
}

impl SseClientTransport {
    /// 创建新的 SSE 客户端传输层
    pub async fn new(config: SseConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;

        let next_id = Arc::new(AtomicU64::new(1));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (notification_tx, _) = broadcast::channel(64);

        let sse_task = {
            let client = client.clone();
            let endpoint = config.endpoint.clone();
            let headers = config.headers.clone();
            let pending_clone = pending.clone();
            let notification_tx_clone = notification_tx.clone();

            tokio::spawn(async move {
                Self::run_sse_loop(
                    client,
                    &endpoint,
                    &headers,
                    pending_clone,
                    notification_tx_clone,
                )
                .await;
            })
        };

        // 等待 SSE 连接建立
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        Ok(Self {
            client,
            config,
            next_id,
            pending,
            notification_tx,
            _sse_task: Some(sse_task),
        })
    }

    async fn run_sse_loop(
        client: reqwest::Client,
        endpoint: &str,
        headers: &HashMap<String, String>,
        pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
        notification_tx: broadcast::Sender<JsonRpcNotification>,
    ) {
        let sse_url = format!("{}/sse", endpoint.trim_end_matches('/'));
        let mut retry_ms: u64 = 2000;

        loop {
            debug!("SSE: Connecting to {}", sse_url);

            let mut builder = client
                .get(&sse_url)
                .header("Accept", "text/event-stream")
                .header("Cache-Control", "no-cache")
                .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION);

            for (k, v) in headers {
                builder = builder.header(k, v);
            }

            match builder.send().await {
                Ok(response) => {
                    if !response.status().is_success() {
                        warn!("SSE: Connection failed with status {}", response.status());
                        tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                        continue;
                    }

                    info!("SSE: Connected to {}", sse_url);

                    let mut stream = response.bytes_stream();
                    let mut buffer = String::new();

                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(bytes) => {
                                let text = match std::str::from_utf8(&bytes) {
                                    Ok(t) => t,
                                    Err(e) => {
                                        error!("SSE: UTF-8 decode error: {}", e);
                                        continue;
                                    }
                                };

                                buffer.push_str(text);

                                // 解析 SSE 事件（以 \n\n 分隔）
                                while let Some(pos) = buffer.find("\n\n") {
                                    let event_block = buffer[..pos].to_string();
                                    buffer = buffer[pos + 2..].to_string();

                                    if let Some(data) = Self::parse_sse_event(&event_block) {
                                        Self::handle_sse_message(&data, &pending, &notification_tx)
                                            .await;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("SSE: Stream error: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("SSE: Connection error: {}", e);
                }
            }

            debug!("SSE: Reconnecting in {}ms...", retry_ms);
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
        pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
        notification_tx: &broadcast::Sender<JsonRpcNotification>,
    ) {
        debug!("SSE: Received data: {}", data);

        let json: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                debug!("SSE: Non-JSON data ignored: {}", e);
                return;
            }
        };

        // 判断是响应还是通知
        if json.get("id").is_some() && (json.get("result").is_some() || json.get("error").is_some())
        {
            // 响应
            match serde_json::from_value::<JsonRpcResponse>(json) {
                Ok(response) => {
                    if let Some(id_val) = &response.id {
                        let id_u64 = match id_val {
                            Value::Number(n) => n.as_u64().unwrap_or(0),
                            Value::String(s) => s.parse().unwrap_or(0),
                            _ => 0,
                        };

                        let mut pending_guard = pending.lock().await;
                        if let Some(tx) = pending_guard.remove(&id_u64) {
                            let _ = tx.send(response);
                        }
                    }
                }
                Err(e) => {
                    error!("SSE: Failed to parse response: {}", e);
                }
            }
        } else if json.get("method").is_some() {
            // 通知
            match serde_json::from_value::<JsonRpcNotification>(json) {
                Ok(notification) => {
                    debug!("SSE: Received notification: {}", notification.method);
                    let _ = notification_tx.send(notification);
                }
                Err(e) => {
                    error!("SSE: Failed to parse notification: {}", e);
                }
            }
        }
    }

    /// 获取通知接收器
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }
}

#[async_trait]
impl Transport for SseClientTransport {
    async fn send(
        &self,
        _id: Option<Value>,
        method: &str,
        params: Option<Value>,
    ) -> anyhow::Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        // 注册等待 channel
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // 构建请求
        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        // 发送 POST 请求
        let mut builder = self
            .client
            .post(&self.config.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
            .json(&request_body);

        for (k, v) in &self.config.headers {
            builder = builder.header(k, v);
        }

        let response = match builder.send().await {
            Ok(r) => r,
            Err(e) => {
                self.pending.lock().await.remove(&id);
                return Err(anyhow::anyhow!("HTTP request failed: {}", e));
            }
        };

        if !response.status().is_success() {
            self.pending.lock().await.remove(&id);
            return Err(anyhow::anyhow!("HTTP error: {}", response.status()));
        }

        // 等待 SSE 推送响应
        let timeout_ms = self.config.timeout_ms;
        let response =
            match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => return Err(anyhow::anyhow!("Response channel closed")),
                Err(_) => {
                    self.pending.lock().await.remove(&id);
                    return Err(anyhow::anyhow!("Response timeout"));
                }
            };

        Ok(response)
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

        for (k, v) in &self.config.headers {
            builder = builder.header(k, v);
        }

        // 通知是 fire-and-forget
        let _ = builder.send().await;

        Ok(())
    }

    async fn close(&self) -> anyhow::Result<()> {
        // SSE 任务会在 drop 时自动停止
        Ok(())
    }
}
