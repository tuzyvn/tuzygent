pub mod types;

#[cfg(test)]
mod tests;

pub use types::{
    McpContentBlock, McpGatewayError, McpJsonRpcError, McpJsonRpcRequest, McpJsonRpcResponse,
    McpPropertyType, McpServerConfig, McpToolCallParams, McpToolCallResult, McpToolDefinition,
    McpToolSchema, JSONRPC_VERSION, MCP_ERR_EXECUTION_FAILED, MCP_ERR_INVALID_PARAMS,
    MCP_ERR_INVALID_REQUEST, MCP_ERR_METHOD_NOT_FOUND, MCP_ERR_PARSE_ERROR, MCP_ERR_POLICY_BLOCKED,
    MCP_ERR_SERVER_UNAVAILABLE,
};

use agent_domain::{ActionSeverity, AuditEventType, AuditIntegrityError, AuditLedger, GuardrailPolicy};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Trait for executing an approved MCP tool call after passing guardrail and schema checks.
pub trait McpToolHandler: Send + Sync {
    fn execute(&self, params: &McpToolCallParams) -> Result<McpToolCallResult, String>;
}

/// Functional adapter allowing closures to act as `McpToolHandler`s.
pub struct FnMcpToolHandler<F>(pub F)
where
    F: Fn(&McpToolCallParams) -> Result<McpToolCallResult, String> + Send + Sync;

impl<F> McpToolHandler for FnMcpToolHandler<F>
where
    F: Fn(&McpToolCallParams) -> Result<McpToolCallResult, String> + Send + Sync,
{
    fn execute(&self, params: &McpToolCallParams) -> Result<McpToolCallResult, String> {
        (self.0)(params)
    }
}

/// Model Context Protocol (MCP) Tool Gateway & Guardrail Policy Interceptor.
///
/// Standardizes MCP JSON-RPC 2.0 requests/responses, validates tool schemas and arguments,
/// intercepts tool invocations against `GuardrailPolicy` red-lines and tool blacklists,
/// and records a tamper-evident cryptographic audit trail in `AuditLedger`.
pub struct McpToolGateway {
    servers: HashMap<String, McpServerConfig>,
    tools: HashMap<String, McpToolDefinition>,
    handlers: HashMap<String, Arc<dyn McpToolHandler>>,
    guardrail: GuardrailPolicy,
    blocked_tools: HashSet<String>,
    audit_ledger: AuditLedger,
}

impl McpToolGateway {
    pub fn new(guardrail: GuardrailPolicy) -> Self {
        Self {
            servers: HashMap::new(),
            tools: HashMap::new(),
            handlers: HashMap::new(),
            guardrail,
            blocked_tools: HashSet::new(),
            audit_ledger: AuditLedger::new(),
        }
    }

    pub fn default_enterprise() -> Self {
        Self::new(GuardrailPolicy::default_enterprise())
    }

    pub fn with_blocked_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for t in tools {
            self.blocked_tools.insert(t.into().to_lowercase());
        }
        self
    }

    pub fn register_server(&mut self, config: McpServerConfig) {
        for blocked in &config.blocked_tools {
            self.blocked_tools.insert(blocked.to_lowercase());
            self.blocked_tools
                .insert(format!("{}:{}", config.server_id.to_lowercase(), blocked.to_lowercase()));
        }
        self.servers.insert(config.server_id.clone(), config);
    }

    pub fn register_tool(&mut self, tool: McpToolDefinition) {
        self.tools.insert(tool.name.clone(), tool);
    }

    pub fn register_tool_with_handler(
        &mut self,
        tool: McpToolDefinition,
        handler: Arc<dyn McpToolHandler>,
    ) {
        let name = tool.name.clone();
        self.tools.insert(name.clone(), tool);
        self.handlers.insert(name, handler);
    }

    pub fn block_tool(&mut self, tool_name: impl Into<String>) {
        self.blocked_tools.insert(tool_name.into().to_lowercase());
    }

    pub fn unblock_tool(&mut self, tool_name: &str) -> bool {
        self.blocked_tools.remove(&tool_name.to_lowercase())
    }

    pub fn guardrail(&self) -> &GuardrailPolicy {
        &self.guardrail
    }

    pub fn guardrail_mut(&mut self) -> &mut GuardrailPolicy {
        &mut self.guardrail
    }

    pub fn audit_ledger(&self) -> &AuditLedger {
        &self.audit_ledger
    }

    pub fn audit_ledger_mut(&mut self) -> &mut AuditLedger {
        &mut self.audit_ledger
    }

    pub fn verify_audit_integrity(&self) -> Result<(), AuditIntegrityError> {
        self.audit_ledger.verify_integrity()
    }

    pub fn is_tool_blacklisted(&self, server_id: &str, tool_name: &str) -> bool {
        if self.blocked_tools.is_empty() {
            return false;
        }
        let lower_name = tool_name.to_lowercase();
        if self.blocked_tools.contains(&lower_name) {
            return true;
        }
        let qualified = format!("{}:{}", server_id.to_lowercase(), lower_name);
        self.blocked_tools.contains(&qualified)
    }

    pub fn list_enabled_tools(&self) -> Vec<&McpToolDefinition> {
        let mut list: Vec<&McpToolDefinition> = self
            .tools
            .values()
            .filter(|t| {
                let server_enabled = self
                    .servers
                    .get(&t.server_id)
                    .map(|s| s.enabled)
                    .unwrap_or(true);
                server_enabled && !self.is_tool_blacklisted(&t.server_id, &t.name)
            })
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Validates tool arguments against the declared `McpToolSchema`.
    pub fn validate_tool_arguments(
        &self,
        tool: &McpToolDefinition,
        args: &Value,
    ) -> Result<(), String> {
        let schema = &tool.input_schema;
        let Some(obj) = args.as_object() else {
            return Err(format!(
                "Tool '{}' requires a JSON object for arguments, found {}",
                tool.name, args
            ));
        };

        for req_key in &schema.required {
            match obj.get(req_key) {
                None | Some(Value::Null) => {
                    return Err(format!(
                        "Missing required argument '{}' for tool '{}'",
                        req_key, tool.name
                    ));
                }
                Some(_) => {}
            }
        }

        for (key, val) in obj {
            if val.is_null() {
                continue;
            }
            if let Some(expected_type) = schema.properties.get(key) {
                if !expected_type.matches(val) {
                    return Err(format!(
                        "Invalid type for argument '{}' on tool '{}': expected {}, got {}",
                        key,
                        tool.name,
                        expected_type.as_str(),
                        val
                    ));
                }
            } else if schema.disallow_additional_properties {
                return Err(format!(
                    "Unknown argument '{}' is not allowed by strict schema for tool '{}'",
                    key, tool.name
                ));
            }
        }

        Ok(())
    }

    /// Evaluates tool name, server blacklist, shell policy, and argument payloads against `GuardrailPolicy`.
    pub fn check_policy_interceptor(
        &self,
        server_id: &str,
        tool: Option<&McpToolDefinition>,
        tool_name: &str,
        args: &Value,
    ) -> Result<(), String> {
        if self.is_tool_blacklisted(server_id, tool_name) {
            return Err(format!(
                "BLACKLIST_BLOCKED: MCP tool '{}' on server '{}' is explicitly blacklisted by policy",
                tool_name, server_id
            ));
        }

        let lower_name = tool_name.to_lowercase();
        let is_shell = tool.map(|t| t.is_shell_tool).unwrap_or(false)
            || lower_name.contains("shell")
            || lower_name.contains("bash")
            || lower_name.contains("exec_cmd");
        if is_shell && !self.guardrail.allow_shell_execution {
            return Err(format!(
                "SHELL_POLICY_BLOCKED: Shell execution via MCP tool '{}' is prohibited by GuardrailPolicy",
                tool_name
            ));
        }

        self.guardrail.check_command_safety(tool_name)?;
        self.inspect_value_safety(args)?;

        Ok(())
    }

    fn inspect_string_safety(&self, s: &str) -> Result<(), String> {
        self.guardrail.check_command_safety(s)?;
        if !self.guardrail.allow_shell_execution {
            let trimmed = s.trim_start();
            let is_shell_cmd = ["bash ", "sh ", "zsh ", "/bin/sh ", "/bin/bash "]
                .iter()
                .any(|prefix| {
                    trimmed
                        .get(..prefix.len())
                        .map(|head| head.eq_ignore_ascii_case(prefix))
                        .unwrap_or(false)
                });
            if is_shell_cmd {
                return Err(format!(
                    "SHELL_POLICY_BLOCKED: Shell command invocation '{}' is prohibited by GuardrailPolicy",
                    s
                ));
            }
        }
        Ok(())
    }

    fn inspect_value_safety(&self, value: &Value) -> Result<(), String> {
        match value {
            Value::String(s) => self.inspect_string_safety(s),
            Value::Array(arr) => {
                for item in arr {
                    self.inspect_value_safety(item)?;
                }
                Ok(())
            }
            Value::Object(map) => {
                for (k, v) in map {
                    self.inspect_string_safety(k)?;
                    self.inspect_value_safety(v)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Intercepts, validates, audits, and executes an MCP `tools/call` invocation.
    pub fn invoke_tool_call(
        &mut self,
        params: McpToolCallParams,
    ) -> Result<McpToolCallResult, McpGatewayError> {
        let tool_opt = self.tools.get(&params.name).cloned();
        let resolved_server_id = params
            .server_id
            .clone()
            .or_else(|| tool_opt.as_ref().map(|t| t.server_id.clone()))
            .unwrap_or_else(|| "default".to_string());

        // 1. Check policy interceptor (blacklist, red-lines, shell execution policy) FIRST
        // so malicious calls even to unknown/blacklisted tools are caught and audited as blocked.
        if let Err(policy_err) = self.check_policy_interceptor(
            &resolved_server_id,
            tool_opt.as_ref(),
            &params.name,
            &params.arguments,
        ) {
            self.record_audit(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                format!(
                    "MCP tools/call blocked [server={}, tool={}]: {}",
                    resolved_server_id, params.name, policy_err
                ),
                params.actor.as_deref(),
            );
            return Err(McpGatewayError::PolicyBlocked(policy_err));
        }

        // 2. Check server availability if registered
        if let Some(server_cfg) = self.servers.get(&resolved_server_id) {
            if !server_cfg.enabled {
                let msg = format!("MCP server '{}' is disabled", resolved_server_id);
                self.record_audit(
                    AuditEventType::CommandBlocked,
                    ActionSeverity::HighImpact,
                    format!(
                        "MCP tools/call rejected on disabled server [server={}, tool={}]",
                        resolved_server_id, params.name
                    ),
                    params.actor.as_deref(),
                );
                return Err(McpGatewayError::ServerUnavailable(msg));
            }
        }

        // 3. Ensure tool is registered
        let Some(tool_def) = tool_opt else {
            let msg = format!(
                "Tool '{}' is not registered on MCP gateway",
                params.name
            );
            self.record_audit(
                AuditEventType::PolicyViolation,
                ActionSeverity::HighImpact,
                format!(
                    "MCP tools/call rejected for unregistered tool [server={}, tool={}]",
                    resolved_server_id, params.name
                ),
                params.actor.as_deref(),
            );
            return Err(McpGatewayError::ToolNotFound(msg));
        };

        // 4. Validate input schema & argument types
        if let Err(schema_err) = self.validate_tool_arguments(&tool_def, &params.arguments) {
            self.record_audit(
                AuditEventType::PolicyViolation,
                ActionSeverity::HighImpact,
                format!(
                    "MCP tools/call schema validation failed [server={}, tool={}]: {}",
                    resolved_server_id, params.name, schema_err
                ),
                params.actor.as_deref(),
            );
            return Err(McpGatewayError::SchemaValidationFailed(schema_err));
        }

        // 5. Execute via handler or default echo responder
        let exec_result = if let Some(handler) = self.handlers.get(&params.name).cloned() {
            handler.execute(&params)
        } else {
            Ok(McpToolCallResult::success_text(format!(
                "Executed MCP tool '{}:{}' with arguments {}",
                resolved_server_id, params.name, params.arguments
            )))
        };

        match exec_result {
            Ok(mut result) => {
                self.enforce_output_limit(&mut result);
                self.record_audit(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!(
                        "MCP tools/call approved and executed [server={}, tool={}, args={}]",
                        resolved_server_id, params.name, params.arguments
                    ),
                    params.actor.as_deref(),
                );
                Ok(result)
            }
            Err(exec_err) => {
                self.record_audit(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!(
                        "MCP tools/call execution exception [server={}, tool={}]: {}",
                        resolved_server_id, params.name, exec_err
                    ),
                    params.actor.as_deref(),
                );
                Err(McpGatewayError::ExecutionFailed(exec_err))
            }
        }
    }

    /// Processes a standardized MCP JSON-RPC 2.0 request envelope.
    pub fn handle_jsonrpc(&mut self, request: McpJsonRpcRequest) -> McpJsonRpcResponse {
        if request.jsonrpc != JSONRPC_VERSION {
            return McpJsonRpcResponse::error(
                request.id,
                MCP_ERR_INVALID_REQUEST,
                format!(
                    "Invalid JSON-RPC version '{}', expected '2.0'",
                    request.jsonrpc
                ),
                None,
            );
        }

        match request.method.as_str() {
            "ping" => McpJsonRpcResponse::success(request.id, json!({ "status": "pong" })),
            "initialize" => McpJsonRpcResponse::success(
                request.id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "tuzygent-mcp-gateway",
                        "version": "1.0.0"
                    },
                    "capabilities": {
                        "tools": { "listChanged": true }
                    }
                }),
            ),
            "tools/list" => {
                let tools: Vec<Value> = self
                    .list_enabled_tools()
                    .into_iter()
                    .map(|t| {
                        json!({
                            "serverId": t.server_id,
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema,
                        })
                    })
                    .collect();
                McpJsonRpcResponse::success(request.id, json!({ "tools": tools }))
            }
            "tools/call" => {
                let Some(raw_params) = request.params else {
                    self.record_audit(
                        AuditEventType::PolicyViolation,
                        ActionSeverity::HighImpact,
                        "MCP tools/call missing required params object".to_string(),
                        None,
                    );
                    return McpJsonRpcResponse::error(
                        request.id,
                        MCP_ERR_INVALID_PARAMS,
                        "Missing 'params' for tools/call",
                        None,
                    );
                };

                let parsed_params: McpToolCallParams = match serde_json::from_value(raw_params) {
                    Ok(p) => p,
                    Err(e) => {
                        self.record_audit(
                            AuditEventType::PolicyViolation,
                            ActionSeverity::HighImpact,
                            format!("MCP tools/call malformed params: {}", e),
                            None,
                        );
                        return McpJsonRpcResponse::error(
                            request.id,
                            MCP_ERR_INVALID_PARAMS,
                            format!("Invalid tools/call parameters: {}", e),
                            None,
                        );
                    }
                };

                match self.invoke_tool_call(parsed_params) {
                    Ok(call_result) => {
                        let val = serde_json::to_value(call_result).unwrap_or(Value::Null);
                        McpJsonRpcResponse::success(request.id, val)
                    }
                    Err(err) => McpJsonRpcResponse::error(
                        request.id,
                        err.jsonrpc_code(),
                        err.to_string(),
                        Some(json!({ "reason": err.to_string() })),
                    ),
                }
            }
            unknown => McpJsonRpcResponse::error(
                request.id,
                MCP_ERR_METHOD_NOT_FOUND,
                format!("MCP method '{}' not found", unknown),
                None,
            ),
        }
    }

    /// Parses and handles a raw JSON string as an MCP JSON-RPC 2.0 request.
    pub fn handle_jsonrpc_str(&mut self, raw_json: &str) -> McpJsonRpcResponse {
        match serde_json::from_str::<McpJsonRpcRequest>(raw_json) {
            Ok(req) => self.handle_jsonrpc(req),
            Err(e) => McpJsonRpcResponse::error(
                Value::Null,
                MCP_ERR_PARSE_ERROR,
                format!("JSON-RPC parse error: {}", e),
                None,
            ),
        }
    }

    fn enforce_output_limit(&self, result: &mut McpToolCallResult) {
        let max_bytes = self.guardrail.max_output_bytes;
        if max_bytes == 0 {
            return;
        }
        for block in &mut result.content {
            if block.text.len() > max_bytes {
                let mut end = max_bytes;
                while end > 0 && !block.text.is_char_boundary(end) {
                    end -= 1;
                }
                block.text.truncate(end);
                block
                    .text
                    .push_str("\n[... output truncated by McpToolGateway guardrail ...]");
            }
        }
    }

    fn record_audit(
        &mut self,
        event_type: AuditEventType,
        severity: ActionSeverity,
        details: String,
        actor: Option<&str>,
    ) {
        if let Some(actor_name) = actor {
            self.audit_ledger.record_with_actor(
                event_type.as_str(),
                severity,
                details,
                actor_name,
            );
        } else {
            self.audit_ledger.record_event(event_type, severity, details);
        }
    }
}
