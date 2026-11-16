use agent_domain::{GuardrailPolicy, Priority, SubTask, Task};
use agent_engine::cli::MockEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::ooda::execution::{
    attempt_remediation_retry, RemediationFailureKind, RemediationOutcome, SubtaskExecutor,
};
use agent_runtime::{
    FnMcpToolHandler, InFlightSupplementController, McpJsonRpcRequest, McpPropertyType,
    McpServerConfig, McpToolCallResult, McpToolDefinition, McpToolGateway, McpToolSchema,
    WorkflowDefinition, WorkflowOrchestrator, MCP_ERR_STALE_REF,
};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn capability_benchmark_stale_ref_recovery_skeleton_reduction_and_dry_run() {
    // 1. Capability: STALE_REF rejection + OODA Re-observe recovery
    let mut gateway = McpToolGateway::new(GuardrailPolicy::default_enterprise());
    gateway.register_server(McpServerConfig::new("fs-mcp", "FS MCP", "stdio://fs-mcp"));
    gateway.register_tool_with_handler(
        McpToolDefinition::new(
            "fs-mcp",
            "patch_node",
            "Patch node with fresh snapshot guard",
            McpToolSchema::new()
                .with_required_property("ref", McpPropertyType::String)
                .with_strict_properties(true),
        )
        .with_requires_fresh_ref(true),
        Arc::new(FnMcpToolHandler(|params| {
            Ok(McpToolCallResult::success_text(format!(
                "patched {}",
                params.arguments["ref"]
            )))
        })),
    );
    gateway.register_snapshot("fs-mcp", "snap_v2");

    let stale_resp = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!(1),
        Some("fs-mcp".to_string()),
        "patch_node",
        json!({ "ref": "@snap_v1:node_a" }),
    ));
    assert_eq!(
        stale_resp.error.as_ref().map(|e| e.code),
        Some(MCP_ERR_STALE_REF)
    );

    let temp_dir =
        std::env::temp_dir().join(format!("tuzygent_cap_bench_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let executor = SubtaskExecutor::new(
        Arc::new(MockEngine::new("")),
        GuardrailPolicy::default(),
        ScratchpadManager::new(temp_dir.join("scratch")),
        DurableMemoryStore::new(temp_dir.join("durable.db")),
        temp_dir.to_string_lossy().to_string(),
        InFlightSupplementController::new(1),
        None,
        None,
        30,
    );
    let mut task = Task::new("CapBench", "StaleRef recovery", Priority::Normal);
    let st = SubTask::new("ST-1", "Patch node", "patch_node @snap_v1:node_a", "ok");
    task.subtasks.push(st.clone());
    let mut spawned = 0;
    let outcome = attempt_remediation_retry(
        &executor,
        &mut task,
        0,
        st,
        "STALE_REF (-32004)",
        RemediationFailureKind::StaleRef,
        &temp_dir,
        "ctx",
        &mut spawned,
    )
    .await;
    assert_eq!(outcome, RemediationOutcome::BacktrackingInserted);
    assert_eq!(task.subtasks[0].id, "ST-1-REOBSERVE-1");

    // Re-acting with fresh snapshot ref succeeds
    let fresh_resp = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!(2),
        Some("fs-mcp".to_string()),
        "patch_node",
        json!({ "ref": "@snap_v2:node_a" }),
    ));
    assert!(fresh_resp.is_ok());

    // 2. Capability: Skeleton token/byte reduction (>= 70%)
    let large_tree = json!({
        "workspace": "tuzygent",
        "crates": {
            "agent-runtime": {
                "modules": {
                    "details": vec!["long-module-metadata-entry-for-benchmarking"; 50]
                }
            }
        }
    });
    let raw_bytes = serde_json::to_string(&large_tree).unwrap().len();
    gateway.register_tool_with_handler(
        McpToolDefinition::new(
            "fs-mcp",
            "dump_tree",
            "Dump workspace tree",
            McpToolSchema::new(),
        ),
        Arc::new(FnMcpToolHandler(move |_| {
            Ok(McpToolCallResult::success_text(
                serde_json::to_string(&large_tree).unwrap(),
            ))
        })),
    );
    let skel_resp = gateway.handle_jsonrpc(McpJsonRpcRequest::tool_call(
        json!(3),
        Some("fs-mcp".to_string()),
        "dump_tree",
        json!({ "_projection": { "skeleton_depth": 2 } }),
    ));
    let skel_bytes = skel_resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .len();
    assert!(
        skel_bytes * 10 <= raw_bytes * 3,
        "expected >=70% reduction: skel_bytes={skel_bytes}, raw_bytes={raw_bytes}"
    );

    // 3. Capability: Workflow dry_run catches type mismatch before execution
    let orchestrator = WorkflowOrchestrator::new(
        temp_dir.to_str().unwrap(),
        temp_dir.to_str().unwrap(),
    );
    let bad_wf: WorkflowDefinition = serde_json::from_value(json!({
        "id": "bench-wf",
        "name": "Typed Edge Bench",
        "nodes": [
            {"id": "a", "type": "triggerNode", "data": {"outputContract": "text"}},
            {"id": "b", "type": "toolNode", "data": {"command": "echo hi", "inputContract": "boolean"}}
        ],
        "edges": [{"id": "e1", "source": "a", "target": "b"}]
    }))
    .unwrap();
    let dry_report = orchestrator.dry_run(&bad_wf, Some(&GuardrailPolicy::default_enterprise()));
    assert!(!dry_report.valid);
    assert!(dry_report
        .errors
        .iter()
        .any(|e| e.contains("EdgeTypeMismatch")));

    let _ = std::fs::remove_dir_all(&temp_dir);
}
