//! STDIO 传输层实现
//!
//! 通过标准输入/输出进行 MCP 消息传输

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::protocol::{
    ERR_INVALID_REQUEST, ERR_PARSE, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
};
use super::Transport;

/// STDIO 传输层
pub struct StdioTransport {
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    running: Arc<AtomicBool>,
}

impl StdioTransport {
    /// 创建新的 STDIO 传输层
    pub fn new() -> Self {
        Self {
            stdout: Arc::new(Mutex::new(tokio::io::stdout())),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// 运行消息循环
    pub async fn run<F, Fut>(&self, mut handler: F) -> anyhow::Result<()>
    where
        F: FnMut(JsonRpcRequest) -> Fut + Send,
        Fut: std::future::Future<Output = JsonRpcResponse> + Send,
    {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        info!("STDIO transport started, waiting for messages...");

        while self.running.load(Ordering::SeqCst) {
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
                    let resp = JsonRpcResponse::error_response(None, ERR_PARSE, &format!("JSON parse error: {}", e));
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
                        let resp = JsonRpcResponse::error_response(None, ERR_INVALID_REQUEST, &format!("Invalid request: {}", e));
                        self.write_response(&resp).await?;
                        continue;
                    }
                };

                let response = handler(request).await;
                self.write_response(&response).await?;
            } else {
                // 通知
                if let Ok(notification) = serde_json::from_value::<JsonRpcNotification>(json) {
                    // 处理通知（通过 handler 传递特殊请求）
                    debug!("STDIO: Received notification: {}", notification.method);
                }
            }
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

    /// 停止传输层
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, _id: Option<Value>, _method: &str, _params: Option<Value>) -> anyhow::Result<JsonRpcResponse> {
        // STDIO 是服务端传输层，不支持主动发送请求
        Err(anyhow::anyhow!("STDIO transport does not support sending requests"))
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> anyhow::Result<()> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        };
        let json = serde_json::to_string(&notification)?;

        let mut stdout = self.stdout.lock().await;
        stdout.write_all(json.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;

        Ok(())
    }

    async fn close(&self) -> anyhow::Result<()> {
        self.stop();
        Ok(())
    }
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}