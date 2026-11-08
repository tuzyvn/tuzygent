use serde::{Deserialize, Serialize};

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
