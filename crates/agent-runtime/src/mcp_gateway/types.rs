use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

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
        let params = McpToolCallParams {
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

/// Configuration for an external MCP server registered with the gateway.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub server_id: String,
    pub name: String,
    pub endpoint: String,
    pub enabled: bool,
    #[serde(default)]
    pub blocked_tools: Vec<String>,
}

impl McpServerConfig {
    pub fn new(
        server_id: impl Into<String>,
        name: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            server_id: server_id.into(),
            name: name.into(),
            endpoint: endpoint.into(),
            enabled: true,
            blocked_tools: Vec::new(),
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn with_blocked_tools(mut self, blocked: Vec<String>) -> Self {
        self.blocked_tools = blocked;
        self
    }
}

/// Supported JSON Schema property types for MCP tool argument validation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpPropertyType {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
}

impl McpPropertyType {
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Array => value.is_array(),
            Self::Object => value.is_object(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

/// Schema specification for an MCP tool's input arguments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolSchema {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub properties: HashMap<String, McpPropertyType>,
    #[serde(default)]
    pub disallow_additional_properties: bool,
}

impl McpToolSchema {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_required_property(
        mut self,
        name: impl Into<String>,
        prop_type: McpPropertyType,
    ) -> Self {
        let key = name.into();
        self.properties.insert(key.clone(), prop_type);
        if !self.required.contains(&key) {
            self.required.push(key);
        }
        self
    }

    pub fn with_optional_property(
        mut self,
        name: impl Into<String>,
        prop_type: McpPropertyType,
    ) -> Self {
        self.properties.insert(name.into(), prop_type);
        self
    }

    pub fn with_strict_properties(mut self, strict: bool) -> Self {
        self.disallow_additional_properties = strict;
        self
    }
}

/// Declaration of an MCP tool exposed by an external MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolDefinition {
    pub server_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: McpToolSchema,
    #[serde(default)]
    pub is_shell_tool: bool,
}

impl McpToolDefinition {
    pub fn new(
        server_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: McpToolSchema,
    ) -> Self {
        Self {
            server_id: server_id.into(),
            name: name.into(),
            description: description.into(),
            input_schema,
            is_shell_tool: false,
        }
    }

    pub fn with_shell_execution(mut self, is_shell: bool) -> Self {
        self.is_shell_tool = is_shell;
        self
    }
}

/// Parameters for an MCP `tools/call` invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolCallParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    pub name: String,
    #[serde(default = "default_empty_object")]
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

fn default_empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Content block returned inside an MCP `tools/call` response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl McpContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: text.into(),
        }
    }
}

/// Standardized result payload for an MCP `tools/call` response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolCallResult {
    pub content: Vec<McpContentBlock>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

impl McpToolCallResult {
    pub fn success_text(text: impl Into<String>) -> Self {
        Self {
            content: vec![McpContentBlock::text(text)],
            is_error: false,
        }
    }

    pub fn error_text(text: impl Into<String>) -> Self {
        Self {
            content: vec![McpContentBlock::text(text)],
            is_error: true,
        }
    }
}

/// Errors produced by `McpToolGateway` during request validation, policy interception, or execution.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum McpGatewayError {
    #[error("MCP Server unavailable or disabled: {0}")]
    ServerUnavailable(String),

    #[error("MCP Tool not found: {0}")]
    ToolNotFound(String),

    #[error("MCP Schema/Argument validation failed: {0}")]
    SchemaValidationFailed(String),

    #[error("MCP Tool call blocked by GuardrailPolicy: {0}")]
    PolicyBlocked(String),

    #[error("MCP Tool execution failed: {0}")]
    ExecutionFailed(String),
}

impl McpGatewayError {
    pub fn jsonrpc_code(&self) -> i32 {
        match self {
            Self::ServerUnavailable(_) | Self::ToolNotFound(_) => MCP_ERR_SERVER_UNAVAILABLE,
            Self::SchemaValidationFailed(_) => MCP_ERR_INVALID_PARAMS,
            Self::PolicyBlocked(_) => MCP_ERR_POLICY_BLOCKED,
            Self::ExecutionFailed(_) => MCP_ERR_EXECUTION_FAILED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        McpGatewayError, McpJsonRpcResponse, MCP_ERR_EXECUTION_FAILED, MCP_ERR_INVALID_PARAMS,
        MCP_ERR_POLICY_BLOCKED, MCP_ERR_SERVER_UNAVAILABLE,
    };
    use serde_json::{json, Value};

    #[test]
    fn jsonrpc_response_status_helpers_reflect_response_state() {
        let success = McpJsonRpcResponse::success(json!(7), json!({ "ok": true }));
        assert!(success.is_ok());
        assert!(!success.is_err());

        let error = McpJsonRpcResponse::error(json!(8), -32600, "invalid request", None);
        assert!(!error.is_ok());
        assert!(error.is_err());

        let empty = McpJsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Value::Null,
            result: None,
            error: None,
        };
        assert!(!empty.is_ok());
        assert!(!empty.is_err());
    }

    #[test]
    fn gateway_errors_map_to_their_jsonrpc_codes() {
        assert_eq!(
            McpGatewayError::ServerUnavailable("offline".to_string()).jsonrpc_code(),
            MCP_ERR_SERVER_UNAVAILABLE
        );
        assert_eq!(
            McpGatewayError::ToolNotFound("missing".to_string()).jsonrpc_code(),
            MCP_ERR_SERVER_UNAVAILABLE
        );
        assert_eq!(
            McpGatewayError::SchemaValidationFailed("invalid".to_string()).jsonrpc_code(),
            MCP_ERR_INVALID_PARAMS
        );
        assert_eq!(
            McpGatewayError::PolicyBlocked("blocked".to_string()).jsonrpc_code(),
            MCP_ERR_POLICY_BLOCKED
        );
        assert_eq!(
            McpGatewayError::ExecutionFailed("failed".to_string()).jsonrpc_code(),
            MCP_ERR_EXECUTION_FAILED
        );
    }
}
