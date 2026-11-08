//! Public type facade for MCP gateway protocol, configuration, and tool payloads.

pub mod error;
pub mod jsonrpc;
pub mod server;
pub mod tool;
pub mod tool_call;

pub use error::McpGatewayError;
pub use jsonrpc::{
    McpJsonRpcError, McpJsonRpcRequest, McpJsonRpcResponse, JSONRPC_VERSION,
    MCP_ERR_EXECUTION_FAILED, MCP_ERR_INVALID_PARAMS, MCP_ERR_INVALID_REQUEST,
    MCP_ERR_METHOD_NOT_FOUND, MCP_ERR_PARSE_ERROR, MCP_ERR_POLICY_BLOCKED,
    MCP_ERR_SERVER_UNAVAILABLE,
};
pub use server::McpServerConfig;
pub use tool::{McpPropertyType, McpToolDefinition, McpToolSchema};
pub use tool_call::{McpContentBlock, McpToolCallParams, McpToolCallResult};

#[cfg(test)]
mod tests {
    use super::{
        McpGatewayError, McpJsonRpcResponse, McpToolCallParams, MCP_ERR_EXECUTION_FAILED,
        MCP_ERR_INVALID_PARAMS, MCP_ERR_POLICY_BLOCKED, MCP_ERR_SERVER_UNAVAILABLE,
    };
    use serde_json::{json, Value};

    #[test]
    fn tool_call_params_default_missing_arguments_to_empty_object() {
        let params: McpToolCallParams = serde_json::from_value(json!({ "name": "read_file" }))
            .expect("tool call params should deserialize without arguments");

        assert_eq!(params.arguments, json!({}));
        assert_eq!(params.server_id, None);
        assert_eq!(params.actor, None);
    }

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
