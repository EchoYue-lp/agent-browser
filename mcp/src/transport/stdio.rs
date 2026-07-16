//! STDIO 传输层实现
//!
//! 通过标准输入/输出进行 MCP 消息传输

use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, error, info};

use crate::protocol::{
    ERR_INVALID_REQUEST, ERR_PARSE, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
};

/// STDIO 传输层
#[derive(Clone)]
pub struct StdioTransport {
    stdout: Arc<Mutex<tokio::io::Stdout>>,
}

impl StdioTransport {
    /// 创建新的 STDIO 传输层
    pub fn new() -> Self {
        Self {
            stdout: Arc::new(Mutex::new(tokio::io::stdout())),
        }
    }

    /// 运行消息循环
    pub async fn run<F, Fut, N>(
        &self,
        handler: F,
        mut notification_handler: N,
    ) -> anyhow::Result<()>
    where
        F: Fn(JsonRpcRequest) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = JsonRpcResponse> + Send + 'static,
        N: FnMut(JsonRpcNotification) + Send,
    {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        let pending = Arc::new(Mutex::new(
            HashMap::<String, tokio::task::AbortHandle>::new(),
        ));

        info!("STDIO transport started, waiting for messages...");

        loop {
            let line = match lines.next_line().await {
                Ok(Some(l)) => l,
                Ok(None) => {
                    debug!("STDIO: EOF reached");
                    break;
                }
                Err(e) => {
                    error!("STDIO: Error reading line: {}", e);
                    break;
                }
            };

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            debug!("STDIO: Received: {}", line);

            // 解析 JSON
            let json: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    error!("STDIO: JSON parse error: {}", e);
                    let resp = JsonRpcResponse::error_response(
                        None,
                        ERR_PARSE,
                        &format!("JSON parse error: {}", e),
                    );
                    self.write_response(&resp).await?;
                    continue;
                }
            };

            // 判断是请求还是通知
            if json.get("id").is_some() {
                // 请求
                let request: JsonRpcRequest = match serde_json::from_value(json) {
                    Ok(r) => r,
                    Err(e) => {
                        error!("STDIO: Invalid request: {}", e);
                        let resp = JsonRpcResponse::error_response(
                            None,
                            ERR_INVALID_REQUEST,
                            &format!("Invalid request: {}", e),
                        );
                        self.write_response(&resp).await?;
                        continue;
                    }
                };

                let request_key = request.id.as_ref().map(request_id_key);
                let request_handler = handler.clone();
                let transport = self.clone();
                let task_pending = pending.clone();
                let (start_tx, start_rx) = oneshot::channel();
                let task_key = request_key.clone();
                let task = tokio::spawn(async move {
                    // Ensure the abort handle is registered before execution can finish.
                    let _ = start_rx.await;
                    let response = request_handler(request).await;
                    if let Err(error) = transport.write_response(&response).await {
                        error!("STDIO: Failed to write response: {error}");
                    }
                    if let Some(key) = task_key {
                        task_pending.lock().await.remove(&key);
                    }
                });
                if let Some(key) = request_key {
                    pending.lock().await.insert(key, task.abort_handle());
                }
                let _ = start_tx.send(());
            } else {
                // 通知
                if let Ok(notification) = serde_json::from_value::<JsonRpcNotification>(json) {
                    debug!("STDIO: Received notification: {}", notification.method);
                    if notification.method == "notifications/cancelled"
                        && let Some(request_id) = notification
                            .params
                            .as_ref()
                            .and_then(|params| params.get("requestId"))
                    {
                        let key = request_id_key(request_id);
                        if let Some(handle) = pending.lock().await.remove(&key) {
                            handle.abort();
                            debug!("STDIO: Aborted request {key}");
                        }
                    }
                    notification_handler(notification);
                }
            }
        }

        for (_, handle) in pending.lock().await.drain() {
            handle.abort();
        }

        info!("STDIO transport stopped");
        Ok(())
    }

    /// 写入响应
    async fn write_response(&self, response: &JsonRpcResponse) -> anyhow::Result<()> {
        let json = serde_json::to_string(response)?;
        debug!("STDIO: Sending: {}", json);

        let mut stdout = self.stdout.lock().await;
        stdout.write_all(json.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;

        Ok(())
    }

    /// Write a server-initiated JSON-RPC notification.
    pub async fn write_notification(
        &self,
        notification: &JsonRpcNotification,
    ) -> anyhow::Result<()> {
        let json = serde_json::to_string(notification)?;
        debug!("STDIO: Sending notification: {}", json);

        let mut stdout = self.stdout.lock().await;
        stdout.write_all(json.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
        Ok(())
    }
}

fn request_id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| id.to_string())
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}
