use agent_domain::{discover_workspace_roster, GuardrailPolicy};
use agent_runtime::{
    FnMcpToolHandler, McpPropertyType, McpServerConfig, McpToolCallResult, McpToolDefinition,
    McpToolGateway, McpToolSchema, WorkflowDefinition, WorkflowOrchestrator,
};
use serde_json::json;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;

/// Builds a `McpToolGateway` exposing core `tuzygent` capabilities over MCP.
pub fn build_tuzygent_mcp_gateway(workspace: &str) -> McpToolGateway {
    let mut gateway = McpToolGateway::new(GuardrailPolicy::default_enterprise());
    gateway.register_server(McpServerConfig::new(
        "tuzygent",
        "Tuzygent Autonomous Multi-Agent Runtime",
        "stdio://agent-ctl",
    ));

    let ws_targets = workspace.to_string();
    gateway.register_tool_with_handler(
        McpToolDefinition::new(
            "tuzygent",
            "tuzygent_rig_list_targets",
            "List discovered Markdown subagent targets in the workspace",
            McpToolSchema::new(),
        ),
        Arc::new(FnMcpToolHandler(move |_| {
            let (roster, _rig_spec) = discover_workspace_roster(Path::new(&ws_targets))?;
            let snapshot_id = format!("snap_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"));
            let targets: Vec<_> = roster
                .iter()
                .map(|spec| {
                    json!({
                        "id": spec.id,
                        "name": spec.name,
                        "description": spec.description,
                        "capabilities": spec.capabilities,
                        "ref": format!("@{}:{}", snapshot_id, spec.id),
                    })
                })
                .collect();
            let payload = json!({
                "snapshot_id": snapshot_id,
                "targets": targets,
            });
            Ok(McpToolCallResult::success_text(payload.to_string()))
        })),
    );

    let ws_dry = workspace.to_string();
    gateway.register_tool_with_handler(
        McpToolDefinition::new(
            "tuzygent",
            "tuzygent_workflow_dry_run",
            "Validate a workflow DAG (typed edges, topological order, guardrail policy) without side effects",
            McpToolSchema::new()
                .with_required_property("workflow", McpPropertyType::Object)
                .with_strict_properties(true),
        ),
        Arc::new(FnMcpToolHandler(move |params| {
            let wf_val = params
                .arguments
                .get("workflow")
                .cloned()
                .ok_or_else(|| "Missing 'workflow' object".to_string())?;
            let wf: WorkflowDefinition = serde_json::from_value(wf_val)
                .map_err(|e| format!("Invalid WorkflowDefinition: {e}"))?;
            let orchestrator = WorkflowOrchestrator::new(&ws_dry, &ws_dry);
            let policy = GuardrailPolicy::default_enterprise();
            let report = orchestrator.dry_run(&wf, Some(&policy));
            let text = serde_json::to_string(&report)
                .map_err(|e| format!("Failed to serialize DryRunReport: {e}"))?;
            Ok(McpToolCallResult::success_text(text))
        })),
    );

    gateway
}

/// Runs the line-delimited JSON-RPC 2.0 MCP server loop over `reader` and `writer`.
pub fn run_stdio_loop<R: BufRead, W: Write>(
    gateway: &mut McpToolGateway,
    reader: R,
    mut writer: W,
) -> anyhow::Result<()> {
    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Ignore JSON-RPC notifications (no `id` or `notifications/*` method) per MCP stdio spec
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let is_notification = val.get("id").is_none()
                && val
                    .get("method")
                    .and_then(|m| m.as_str())
                    .is_some_and(|m| m.starts_with("notifications/"));
            if is_notification {
                continue;
            }
        }

        let response = gateway.handle_jsonrpc_str(trimmed);
        let serialized = serde_json::to_string(&response)?;
        writeln!(writer, "{serialized}")?;
        writer.flush()?;
    }
    Ok(())
}

/// Entry point for `agent-ctl mcp`.
pub async fn handle(workspace: String) -> anyhow::Result<()> {
    let mut gateway = build_tuzygent_mcp_gateway(&workspace);
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_stdio_loop(&mut gateway, stdin.lock(), stdout.lock())
}
