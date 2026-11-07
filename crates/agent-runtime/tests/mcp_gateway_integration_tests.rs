use agent_domain::{
    ActionSeverity, AuditEventType, AuditIntegrityError, AuditLedger, GuardrailPolicy, RedLineRule,
    GENESIS_HASH,
};
use agent_runtime::{
    FnMcpToolHandler, McpGatewayError, McpJsonRpcRequest, McpPropertyType, McpServerConfig,
    McpToolCallParams, McpToolCallResult, McpToolDefinition, McpToolGateway, McpToolSchema,
    MCP_ERR_EXECUTION_FAILED, MCP_ERR_INVALID_PARAMS, MCP_ERR_INVALID_REQUEST,
    MCP_ERR_METHOD_NOT_FOUND, MCP_ERR_PARSE_ERROR, MCP_ERR_POLICY_BLOCKED,
    MCP_ERR_SERVER_UNAVAILABLE,
};
use serde_json::json;
use std::sync::Arc;

fn build_integration_mcp_gateway() -> McpToolGateway {
    let mut policy = GuardrailPolicy::default_enterprise();
    policy.red_lines.push(RedLineRule {
        pattern: "curl http://169.254.169.254|metadata.google.internal".to_string(),
        description: "Cloud instance metadata SSRF attempt".to_string(),
        severity: ActionSeverity::Destructive,
        requires_human_approval: true,
    });

    let mut gateway = McpToolGateway::new(policy)
        .with_blocked_tools(["raw_kernel_module_load", "dump_process_memory"]);

    gateway.register_server(
        McpServerConfig::new("git-mcp", "Git Workspace MCP Server", "stdio://git-mcp")
            .with_blocked_tools(vec!["git_force_push_master".to_string()]),
    );
    gateway.register_server(McpServerConfig::new(
        "postgres-mcp",
        "Postgres Analytics MCP Server",
        "http://127.0.0.1:8081/mcp",
    ));
    gateway.register_server(
        McpServerConfig::new(
            "legacy-mcp",
            "Disabled Legacy MCP Server",
            "http://127.0.0.1:8089/mcp",
        )
        .with_enabled(false),
    );

    let git_diff_schema = McpToolSchema::new()
        .with_required_property("repo_path", McpPropertyType::String)
        .with_optional_property("staged", McpPropertyType::Boolean)
        .with_optional_property("context_lines", McpPropertyType::Integer)
        .with_strict_properties(true);

    gateway.register_tool_with_handler(
        McpToolDefinition::new(
            "git-mcp",
            "git_diff",
            "Inspect git diff for a repository path",
            git_diff_schema,
        ),
        Arc::new(FnMcpToolHandler(|params| {
            let repo = params
                .arguments
                .get("repo_path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            let staged = params
                .arguments
                .get("staged")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(McpToolCallResult::success_text(format!(
                "diff --git a/{repo} b/{repo} (staged={staged})"
            )))
        })),
    );

    let sql_schema = McpToolSchema::new()
        .with_required_property("sql", McpPropertyType::String)
        .with_optional_property("params", McpPropertyType::Array)
        .with_strict_properties(true);

    gateway.register_tool_with_handler(
        McpToolDefinition::new(
            "postgres-mcp",
            "execute_sql",
            "Run parameterized SQL query",
            sql_schema,
        ),
        Arc::new(FnMcpToolHandler(|params| {
            let sql = params
                .arguments
                .get("sql")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if sql.contains("SIMULATE_DB_CRASH") {
                return Err("Database connection lost during query".to_string());
            }
            Ok(McpToolCallResult::success_text(format!(
                "rows_returned: 2 for [{sql}]"
            )))
        })),
    );

    let shell_schema = McpToolSchema::new()
        .with_required_property("cmd", McpPropertyType::String)
        .with_optional_property("env", McpPropertyType::Object);

    gateway.register_tool(
        McpToolDefinition::new(
            "git-mcp",
            "workspace_shell",
            "Run shell command inside workspace",
            shell_schema,
        )
        .with_shell_execution(true),
    );

    gateway.register_tool(McpToolDefinition::new(
        "legacy-mcp",
        "legacy_ping",
        "Ping disabled server",
        McpToolSchema::new(),
    ));

    gateway
}

#[test]
fn integration_mcp_allowed_tool_calls_and_discovery() {
    let mut gateway = build_integration_mcp_gateway();

    let init_resp = gateway.handle_jsonrpc(McpJsonRpcRequest::new(json!(1), "initialize", None));
    assert!(init_resp.is_ok());
    assert_eq!(
        init_resp.result.as_ref().map(|r| &r["serverInfo"]["name"]),
        Some(&json!("tuzygent-mcp-gateway"))
    );

    let list_resp = gateway.handle_jsonrpc(McpJsonRpcRequest::new(json!(2), "tools/list", None));
    assert!(list_resp.is_ok());
    let tools_arr = list_resp
        .result
        .as_ref()
        .and_then(|r| r["tools"].as_array())
        .expect("tools array");
    assert_eq!(tools_arr.len(), 3);

    let call_git = McpJsonRpcRequest::tool_call(
        json!(3),
        Some("git-mcp".to_string()),
        "git_diff",
        json!({
            "repo_path": "crates/agent-runtime",
            "staged": true,
            "context_lines": 3
        }),
    );
    let git_resp = gateway.handle_jsonrpc(call_git);
    assert!(git_resp.is_ok());
    let git_res = git_resp.result.expect("git result");
    assert_eq!(git_res["isError"], json!(false));
    assert_eq!(
        git_res["content"][0]["text"],
        json!("diff --git a/crates/agent-runtime b/crates/agent-runtime (staged=true)")
    );

    let call_sql = McpJsonRpcRequest::tool_call(
        json!(4),
        Some("postgres-mcp".to_string()),
        "execute_sql",
        json!({
            "sql": "SELECT id, title FROM tasks WHERE status = $1",
            "params": ["Completed"]
        }),
    );
    let sql_resp = gateway.handle_jsonrpc(call_sql);
    assert!(sql_resp.is_ok());

    assert_eq!(gateway.audit_ledger().len(), 2);
    assert!(gateway
        .audit_ledger()
        .entries()
        .iter()
        .all(|e| e.event_type == AuditEventType::CommandAllowed));
    assert!(gateway.verify_audit_integrity().is_ok());
}

#[test]
fn integration_mcp_blocked_tool_calls_violating_guardrail_policy() {
    let mut gateway = build_integration_mcp_gateway();

    let resp_global_block = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!("req-1"),
        Some("git-mcp".to_string()),
        "raw_kernel_module_load",
        json!({}),
    ));
    assert!(resp_global_block.is_err());
    assert_eq!(
        resp_global_block.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    let resp_server_block = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!("req-2"),
        Some("git-mcp".to_string()),
        "git_force_push_master",
        json!({}),
    ));
    assert!(resp_server_block.is_err());
    assert_eq!(
        resp_server_block.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    let resp_sql_drop = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!("req-3"),
        Some("postgres-mcp".to_string()),
        "execute_sql",
        json!({ "sql": "DROP TABLE audit_ledger;" }),
    ));
    assert!(resp_sql_drop.is_err());
    assert_eq!(
        resp_sql_drop.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    let resp_rm = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!("req-4"),
        Some("git-mcp".to_string()),
        "workspace_shell",
        json!({ "cmd": "rm -rf /etc && chmod 777 /var" }),
    ));
    assert!(resp_rm.is_err());
    assert_eq!(
        resp_rm.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    let resp_ssrf = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!("req-5"),
        Some("git-mcp".to_string()),
        "workspace_shell",
        json!({
            "cmd": "echofetching",
            "env": { "HOOK": "curl http://169.254.169.254/latest/meta-data" }
        }),
    ));
    assert!(resp_ssrf.is_err());
    assert_eq!(
        resp_ssrf.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    gateway.guardrail_mut().allow_shell_execution = false;
    let resp_shell_disabled = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!("req-6"),
        Some("git-mcp".to_string()),
        "workspace_shell",
        json!({ "cmd": "git status" }),
    ));
    assert!(resp_shell_disabled.is_err());
    assert_eq!(
        resp_shell_disabled.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_POLICY_BLOCKED)
    );

    let resp_disabled_srv = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!("req-7"),
        Some("legacy-mcp".to_string()),
        "legacy_ping",
        json!({}),
    ));
    assert!(resp_disabled_srv.is_err());
    assert_eq!(
        resp_disabled_srv.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_SERVER_UNAVAILABLE)
    );

    assert_eq!(gateway.audit_ledger().len(), 7);
    assert!(gateway
        .audit_ledger()
        .entries()
        .iter()
        .all(|e| e.event_type == AuditEventType::CommandBlocked));
    assert!(gateway.verify_audit_integrity().is_ok());
}

#[test]
fn integration_mcp_schema_and_argument_validation_errors() {
    let mut gateway = build_integration_mcp_gateway();

    let err_missing = gateway
        .invoke_tool_call(McpToolCallParams {
            server_id: Some("git-mcp".to_string()),
            name: "git_diff".to_string(),
            arguments: json!({ "staged": false }),
            actor: Some("agent-coder".to_string()),
        })
        .expect_err("Missing required repo_path must fail validation");
    assert!(matches!(
        err_missing,
        McpGatewayError::SchemaValidationFailed(_)
    ));

    let err_type = gateway
        .invoke_tool_call(McpToolCallParams {
            server_id: Some("git-mcp".to_string()),
            name: "git_diff".to_string(),
            arguments: json!({
                "repo_path": "src",
                "staged": "yes"
            }),
            actor: Some("agent-coder".to_string()),
        })
        .expect_err("Wrong boolean type must fail validation");
    assert!(matches!(
        err_type,
        McpGatewayError::SchemaValidationFailed(_)
    ));

    let err_extra = gateway
        .invoke_tool_call(McpToolCallParams {
            server_id: Some("git-mcp".to_string()),
            name: "git_diff".to_string(),
            arguments: json!({
                "repo_path": "src",
                "unknown_param": 123
            }),
            actor: Some("agent-coder".to_string()),
        })
        .expect_err("Unexpected property must fail strict validation");
    assert!(matches!(
        err_extra,
        McpGatewayError::SchemaValidationFailed(_)
    ));

    let err_non_obj = gateway
        .invoke_tool_call(McpToolCallParams {
            server_id: Some("git-mcp".to_string()),
            name: "git_diff".to_string(),
            arguments: json!(["not", "an", "object"]),
            actor: None,
        })
        .expect_err("Array arguments must be rejected");
    assert!(matches!(
        err_non_obj,
        McpGatewayError::SchemaValidationFailed(_)
    ));

    let err_not_found = gateway
        .invoke_tool_call(McpToolCallParams {
            server_id: Some("git-mcp".to_string()),
            name: "nonexistent_tool".to_string(),
            arguments: json!({}),
            actor: None,
        })
        .expect_err("Unregistered tool must return ToolNotFound");
    assert!(matches!(err_not_found, McpGatewayError::ToolNotFound(_)));

    let parse_err_resp = gateway.handle_jsonrpc_str("{ malformed json ");
    assert_eq!(
        parse_err_resp.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_PARSE_ERROR)
    );

    let bad_ver_resp = gateway.handle_jsonrpc(McpJsonRpcRequest {
        jsonrpc: "1.0".to_string(),
        id: json!(99),
        method: "tools/list".to_string(),
        params: None,
    });
    assert_eq!(
        bad_ver_resp.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_INVALID_REQUEST)
    );

    let unknown_method_resp =
        gateway.handle_jsonrpc(McpJsonRpcRequest::new(json!(100), "resources/purge", None));
    assert_eq!(
        unknown_method_resp.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_METHOD_NOT_FOUND)
    );

    let invalid_params_resp =
        gateway.handle_jsonrpc(McpJsonRpcRequest::new(json!(101), "tools/call", None));
    assert_eq!(
        invalid_params_resp.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_INVALID_PARAMS)
    );

    assert_eq!(gateway.audit_ledger().len(), 6);
    assert!(gateway.verify_audit_integrity().is_ok());
}

#[test]
fn integration_mcp_audit_ledger_hash_chain_and_tamper_detection() {
    let mut gateway = build_integration_mcp_gateway();

    let ok_res = gateway.invoke_tool_call(McpToolCallParams {
        server_id: Some("git-mcp".to_string()),
        name: "git_diff".to_string(),
        arguments: json!({ "repo_path": "crates/agent-domain" }),
        actor: Some("agent-auditor".to_string()),
    });
    assert!(ok_res.is_ok());

    let blocked_res = gateway.invoke_tool_call(McpToolCallParams {
        server_id: Some("postgres-mcp".to_string()),
        name: "execute_sql".to_string(),
        arguments: json!({ "sql": "TRUNCATE tasks;" }),
        actor: Some("agent-rogue".to_string()),
    });
    assert!(matches!(
        blocked_res,
        Err(McpGatewayError::PolicyBlocked(_))
    ));

    let schema_res = gateway.invoke_tool_call(McpToolCallParams {
        server_id: Some("postgres-mcp".to_string()),
        name: "execute_sql".to_string(),
        arguments: json!({ "params": [1, 2] }),
        actor: Some("agent-worker".to_string()),
    });
    assert!(matches!(
        schema_res,
        Err(McpGatewayError::SchemaValidationFailed(_))
    ));

    let exec_err_resp = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!(500),
        Some("postgres-mcp".to_string()),
        "execute_sql",
        json!({ "sql": "SELECT SIMULATE_DB_CRASH" }),
    ));
    assert_eq!(
        exec_err_resp.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_EXECUTION_FAILED)
    );

    let ledger = gateway.audit_ledger();
    assert_eq!(ledger.len(), 4);
    assert!(gateway.verify_audit_integrity().is_ok());

    let entries = ledger.entries();
    assert_eq!(entries[0].prev_hash, GENESIS_HASH);
    assert_eq!(entries[0].event_type, AuditEventType::CommandAllowed);
    assert_eq!(entries[0].actor.as_deref(), Some("agent-auditor"));

    assert_eq!(entries[1].prev_hash, entries[0].hash);
    assert_eq!(entries[1].event_type, AuditEventType::CommandBlocked);
    assert_eq!(entries[1].severity, ActionSeverity::Destructive);
    assert_eq!(entries[1].actor.as_deref(), Some("agent-rogue"));

    assert_eq!(entries[2].prev_hash, entries[1].hash);
    assert_eq!(entries[2].event_type, AuditEventType::PolicyViolation);

    assert_eq!(entries[3].prev_hash, entries[2].hash);
    assert_eq!(entries[3].event_type, AuditEventType::CommandException);

    let mut tampered = entries.to_vec();
    tampered[1].details = "MCP tools/call approved".to_string();
    let tampered_ledger = AuditLedger::from_entries(tampered);
    assert!(matches!(
        tampered_ledger.verify_integrity(),
        Err(AuditIntegrityError::TamperedHash { sequence: 1, .. })
    ));
}
