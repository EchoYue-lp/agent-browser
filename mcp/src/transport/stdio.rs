//! STDIO 传输层实现
//!
//! 通过标准输入/输出进行 MCP 消息传输

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::protocol::{
    ERR_INVALID_REQUEST, ERR_PARSE, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
};

/// STDIO 传输层
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
        mut handler: F,
        mut notification_handler: N,
    ) -> anyhow::Result<()>
    where
        F: FnMut(JsonRpcRequest) -> Fut + Send,
        Fut: std::future::Future<Output = JsonRpcResponse> + Send,
        N: FnMut(JsonRpcNotification) + Send,
    {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

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

                let response = handler(request).await;
                self.write_response(&response).await?;
            } else {
                // 通知
                if let Ok(notification) = serde_json::from_value::<JsonRpcNotification>(json) {
                    debug!("STDIO: Received notification: {}", notification.method);
                    notification_handler(notification);
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
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}
