use super::*;
use serde_json::json;
use std::sync::Arc;

fn setup_gateway() -> McpToolGateway {
    let mut gateway = McpToolGateway::default_enterprise()
        .with_blocked_tools(["host_root_shell", "exfiltrate_secrets"]);

    gateway.register_server(
        McpServerConfig::new("fs-mcp", "Filesystem MCP Server", "stdio://fs-mcp")
            .with_blocked_tools(vec!["format_disk".to_string()]),
    );
    gateway.register_server(McpServerConfig::new(
        "db-mcp",
        "Database MCP Server",
        "http://127.0.0.1:9001/mcp",
    ));

    let read_schema = McpToolSchema::new()
        .with_required_property("path", McpPropertyType::String)
        .with_optional_property("max_lines", McpPropertyType::Integer)
        .with_strict_properties(true);

    gateway.register_tool_with_handler(
        McpToolDefinition::new(
            "fs-mcp",
            "read_file",
            "Read a workspace file safely",
            read_schema,
        ),
        Arc::new(FnMcpToolHandler(|params| {
            let path = params
                .arguments
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(McpToolCallResult::success_text(format!(
                "file_contents_of:{}",
                path
            )))
        })),
    );

    let sql_schema = McpToolSchema::new()
        .with_required_property("query", McpPropertyType::String)
        .with_optional_property("limit", McpPropertyType::Integer);

    gateway.register_tool(McpToolDefinition::new(
        "db-mcp",
        "query_database",
        "Execute a SQL query against the database",
        sql_schema,
    ));

    let shell_schema =
        McpToolSchema::new().with_required_property("command", McpPropertyType::String);

    gateway.register_tool(
        McpToolDefinition::new(
            "fs-mcp",
            "run_shell_command",
            "Run a shell command in workspace",
            shell_schema,
        )
        .with_shell_execution(true),
    );

    gateway
}

#[test]
fn test_mcp_allowed_tool_call_via_jsonrpc_and_audit_logged() {
    let mut gateway = setup_gateway();

    let req = McpJsonRpcRequest::tool_call(
        json!(1),
        Some("fs-mcp".to_string()),
        "read_file",
        json!({
            "path": "crates/agent-runtime/Cargo.toml",
            "max_lines": 50
        }),
    );

    let resp = gateway.handle_jsonrpc(req);
    assert!(resp.is_ok(), "Safe read_file call should be allowed");
    let res_val = resp.result.expect("result present");
    assert_eq!(res_val["isError"], json!(false));
    assert_eq!(
        res_val["content"][0]["text"],
        json!("file_contents_of:crates/agent-runtime/Cargo.toml")
    );

    assert_eq!(gateway.audit_ledger().len(), 1);
    let entry = &gateway.audit_ledger().entries()[0];
    assert_eq!(entry.event_type, AuditEventType::CommandAllowed);
    assert_eq!(entry.severity, ActionSeverity::Routine);
    assert!(entry.details.contains("read_file"));
    assert!(gateway.verify_audit_integrity().is_ok());
}

#[test]
fn test_mcp_blocked_tool_calls_violating_policy_and_blacklist() {
    let mut gateway = setup_gateway();

    // 1. Explicitly blacklisted tool at gateway level
    let req_blacklisted = McpJsonRpcRequest::tool_call(
        json!(10),
        Some("fs-mcp".to_string()),
        "host_root_shell",
        json!({}),
    );
    let resp1 = gateway.handle_jsonrpc(req_blacklisted);
    assert!(resp1.is_err());
    assert_eq!(
        resp1.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    // 2. Server-level blacklisted tool
    let req_server_blocked = McpJsonRpcRequest::tool_call(
        json!(11),
        Some("fs-mcp".to_string()),
        "format_disk",
        json!({ "device": "/dev/sda" }),
    );
    let resp2 = gateway.handle_jsonrpc(req_server_blocked);
    assert!(resp2.is_err());
    assert_eq!(
        resp2.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    // 3. GuardrailPolicy red-line violation inside SQL argument (DROP TABLE)
    let req_sql_drop = McpJsonRpcRequest::tool_call(
        json!(12),
        Some("db-mcp".to_string()),
        "query_database",
        json!({ "query": "SELECT 1; DROP TABLE users;" }),
    );
    let resp3 = gateway.handle_jsonrpc(req_sql_drop);
    assert!(resp3.is_err());
    assert_eq!(
        resp3.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    // 4. GuardrailPolicy red-line violation inside shell argument (rm -rf)
    let req_rm_rf = McpJsonRpcRequest::tool_call(
        json!(13),
        Some("fs-mcp".to_string()),
        "run_shell_command",
        json!({ "command": "rm -rf /var/lib/postgresql" }),
    );
    let resp4 = gateway.handle_jsonrpc(req_rm_rf);
    assert!(resp4.is_err());
    assert_eq!(
        resp4.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    // 5. Shell execution disabled policy
    gateway.guardrail_mut().allow_shell_execution = false;
    let req_shell_prohibited = McpJsonRpcRequest::tool_call(
        json!(14),
        Some("fs-mcp".to_string()),
        "run_shell_command",
        json!({ "command": "ls -la" }),
    );
    let resp5 = gateway.handle_jsonrpc(req_shell_prohibited);
    assert!(resp5.is_err());
    assert_eq!(
        resp5.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    assert_eq!(gateway.audit_ledger().len(), 5);
    for entry in gateway.audit_ledger().entries() {
        assert_eq!(entry.event_type, AuditEventType::CommandBlocked);
        assert_eq!(entry.severity, ActionSeverity::Destructive);
    }
    assert!(gateway.verify_audit_integrity().is_ok());
}

#[test]
fn test_mcp_schema_and_argument_validation() {
    let mut gateway = setup_gateway();

    // 1. Missing required argument 'path'
    let req_missing = McpJsonRpcRequest::tool_call(
        json!(20),
        Some("fs-mcp".to_string()),
        "read_file",
        json!({ "max_lines": 10 }),
    );
    let resp_missing = gateway.handle_jsonrpc(req_missing);
    assert!(resp_missing.is_err());
    assert_eq!(
        resp_missing.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_INVALID_PARAMS)
    );

    // 2. Wrong argument type (max_lines passed as string instead of integer)
    let req_wrong_type = McpJsonRpcRequest::tool_call(
        json!(21),
        Some("fs-mcp".to_string()),
        "read_file",
        json!({ "path": "README.md", "max_lines": "fifty" }),
    );
    let resp_wrong_type = gateway.handle_jsonrpc(req_wrong_type);
    assert!(resp_wrong_type.is_err());
    assert_eq!(
        resp_wrong_type.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_INVALID_PARAMS)
    );

    // 3. Unknown additional property when strict schema is enabled
    let req_unknown_prop = McpJsonRpcRequest::tool_call(
        json!(22),
        Some("fs-mcp".to_string()),
        "read_file",
        json!({ "path": "README.md", "unexpected_flag": true }),
    );
    let resp_unknown_prop = gateway.handle_jsonrpc(req_unknown_prop);
    assert!(resp_unknown_prop.is_err());
    assert_eq!(
        resp_unknown_prop.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_INVALID_PARAMS)
    );

    assert_eq!(gateway.audit_ledger().len(), 3);
    for entry in gateway.audit_ledger().entries() {
        assert_eq!(entry.event_type, AuditEventType::PolicyViolation);
    }
    assert!(gateway.verify_audit_integrity().is_ok());
}

#[test]
fn test_mcp_edge_cases_obfuscated_redlines_and_null_arguments() {
    let mut gateway = setup_gateway();

    // 1. Obfuscated whitespace and mixed-case red-line inside nested JSON array/object
    let req_obfuscated = McpJsonRpcRequest::tool_call(
        json!(30),
        Some("db-mcp".to_string()),
        "query_database",
        json!({
            "query": "SELECT * FROM logs;\n  dRoP   \t  tAbLe   audit_logs;"
        }),
    );
    let resp_obfuscated = gateway.handle_jsonrpc(req_obfuscated);
    assert!(resp_obfuscated.is_err());
    assert_eq!(
        resp_obfuscated.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    // 2. Null value for required property must be rejected, whereas null on optional property is allowed
    let req_null_required = McpJsonRpcRequest::tool_call(
        json!(31),
        Some("fs-mcp".to_string()),
        "read_file",
        json!({ "path": null }),
    );
    let resp_null_req = gateway.handle_jsonrpc(req_null_required);
    assert!(resp_null_req.is_err());
    assert_eq!(
        resp_null_req.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_INVALID_PARAMS)
    );

    let req_null_optional = McpJsonRpcRequest::tool_call(
        json!(32),
        Some("fs-mcp".to_string()),
        "read_file",
        json!({ "path": "Cargo.toml", "max_lines": null }),
    );
    let resp_null_opt = gateway.handle_jsonrpc(req_null_optional);
    assert!(resp_null_opt.is_ok());

    // 3. Shell command payload smuggled into non-shell tool when allow_shell_execution is false
    gateway.guardrail_mut().allow_shell_execution = false;
    let req_smuggled_shell = McpJsonRpcRequest::tool_call(
        json!(33),
        Some("fs-mcp".to_string()),
        "read_file",
        json!({ "path": "/bin/bash -c 'cat /etc/shadow'" }),
    );
    let resp_smuggled = gateway.handle_jsonrpc(req_smuggled_shell);
    assert!(resp_smuggled.is_err());
    assert_eq!(
        resp_smuggled.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    // 4. Runtime dynamic tool blocking via block_tool() and unblocking via unblock_tool()
    gateway.block_tool("query_database");
    let req_dyn_blocked = McpJsonRpcRequest::tool_call(
        json!(34),
        Some("db-mcp".to_string()),
        "query_database",
        json!({ "query": "SELECT 1" }),
    );
    let resp_dyn = gateway.handle_jsonrpc(req_dyn_blocked);
    assert!(resp_dyn.is_err());
    assert_eq!(
        resp_dyn.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    assert!(gateway.unblock_tool("query_database"));
    gateway.guardrail_mut().max_output_bytes = 20;
    let req_unblocked = McpJsonRpcRequest::tool_call(
        json!(35),
        Some("db-mcp".to_string()),
        "query_database",
        json!({ "query": "SELECT 1" }),
    );
    let resp_unblocked = gateway.handle_jsonrpc(req_unblocked);
    assert!(resp_unblocked.is_ok());
    let out_text = resp_unblocked.result.expect("result")["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(out_text.contains("[... output truncated by McpToolGateway guardrail ...]"));

    assert!(gateway.verify_audit_integrity().is_ok());
}
