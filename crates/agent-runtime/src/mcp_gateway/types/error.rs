use thiserror::Error;

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
            Self::ServerUnavailable(_) | Self::ToolNotFound(_) => super::MCP_ERR_SERVER_UNAVAILABLE,
            Self::SchemaValidationFailed(_) => super::MCP_ERR_INVALID_PARAMS,
            Self::PolicyBlocked(_) => super::MCP_ERR_POLICY_BLOCKED,
            Self::ExecutionFailed(_) => super::MCP_ERR_EXECUTION_FAILED,
        }
    }
}
