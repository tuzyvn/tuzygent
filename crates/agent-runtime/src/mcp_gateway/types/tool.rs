use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

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
