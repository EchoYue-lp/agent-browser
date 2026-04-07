//! MCP 传输层抽象
//!
//! 定义 MCP 消息传输的统一接口

pub mod stdio;
pub mod sse;
pub mod http;

use async_trait::async_trait;
use serde_json::Value;

use crate::protocol::{JsonRpcNotification, JsonRpcResponse};

/// MCP 传输层 trait
#[async_trait]
pub trait Transport: Send + Sync {
    /// 发送请求并等待响应
    async fn send(&self, id: Option<Value>, method: &str, params: Option<Value>) -> anyhow::Result<JsonRpcResponse>;

    /// 发送通知（无需响应）
    async fn notify(&self, method: &str, params: Option<Value>) -> anyhow::Result<()>;

    /// 关闭传输层
    async fn close(&self) -> anyhow::Result<()>;
}