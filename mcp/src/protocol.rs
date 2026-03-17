//! MCP 协议类型定义
//!
//! 简化的 JSON-RPC 2.0 实现

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 请求
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC 版本（必须是 "2.0"）
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// 请求 ID
    pub id: Value,
    /// 方法名
    pub method: String,
    /// 参数
    #[serde(default)]
    pub params: Option<Value>,
}

/// JSON-RPC 响应
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC 版本
    pub jsonrpc: &'static str,
    /// 请求 ID
    pub id: Value,
    /// 结果（成功时）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// 错误（失败时）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 错误
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    /// 错误码
    pub code: i32,
    /// 错误消息
    pub message: String,
}

impl JsonRpcResponse {
    /// 创建成功响应
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    /// 创建错误响应
    pub fn error(id: Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
            }),
        }
    }
}
