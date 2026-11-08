use serde::{Deserialize, Serialize};
use serde_json::Value;

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
