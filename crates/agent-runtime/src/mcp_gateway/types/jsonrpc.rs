use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

pub const MCP_ERR_PARSE_ERROR: i32 = -32700;
pub const MCP_ERR_INVALID_REQUEST: i32 = -32600;
pub const MCP_ERR_METHOD_NOT_FOUND: i32 = -32601;
pub const MCP_ERR_INVALID_PARAMS: i32 = -32602;
pub const MCP_ERR_POLICY_BLOCKED: i32 = -32001;
pub const MCP_ERR_SERVER_UNAVAILABLE: i32 = -32002;
pub const MCP_ERR_EXECUTION_FAILED: i32 = -32003;

/// Standardized MCP JSON-RPC 2.0 Request envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpJsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

impl McpJsonRpcRequest {
    pub fn new(id: impl Into<Value>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }

    pub fn tool_call(
        id: impl Into<Value>,
        server_id: Option<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        let params = super::McpToolCallParams {
            server_id,
            name: tool_name.into(),
            arguments,
            actor: None,
        };
        let params_val = serde_json::to_value(params).unwrap_or(Value::Null);
        Self::new(id, "tools/call", Some(params_val))
    }
}

/// Standardized MCP JSON-RPC 2.0 Error object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpJsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl McpJsonRpcError {
    pub fn new(code: i32, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            code,
            message: message.into(),
            data,
        }
    }
}

/// Standardized MCP JSON-RPC 2.0 Response envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpJsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<McpJsonRpcError>,
}

impl McpJsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i32, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(McpJsonRpcError::new(code, message, data)),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.error.is_none() && self.result.is_some()
    }

    pub fn is_err(&self) -> bool {
        self.error.is_some()
    }
}
