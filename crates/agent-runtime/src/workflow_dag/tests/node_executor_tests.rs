use std::collections::HashMap;
use super::test_workflow;
use crate::workflow_dag::*;

fn make_node(id: &str, node_type: &str, data: serde_json::Value) -> WorkflowNode {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "type": node_type,
        "data": data,
    }))
    .expect("WorkflowNode JSON must deserialize into WorkflowNode")
}

struct TestWorkspace {
    dir: std::path::PathBuf,
    orchestrator: WorkflowOrchestrator,
}

impl TestWorkspace {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("tuzygent-wf-unit-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("Failed to create temp dir");
        let dir_str = dir.to_str().expect("Valid UTF-8 dir path");
        let orchestrator = WorkflowOrchestrator::new(dir_str, dir_str);
        Self { dir, orchestrator }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_execute_trigger_node_branches() {
    // Branch A: Custom triggerType
    let node_custom = make_node(
        "trig_custom",
        "triggerNode",
        serde_json::json!({
            "triggerType": "webhook"
        }),
    );
    let outcome_custom = execute_trigger_node(&node_custom);
    assert_eq!(outcome_custom.status, "completed");
    assert_eq!(outcome_custom.node_output["triggered"], true);
    assert_eq!(outcome_custom.node_output["type"], "webhook");
    assert!(outcome_custom.node_output["timestamp"].is_string());
    assert!(chrono::DateTime::parse_from_rfc3339(
        outcome_custom.node_output["timestamp"].as_str().unwrap()
    ).is_ok());
    assert!(outcome_custom.node_error.is_none());
    assert!(outcome_custom.engine_used.is_none());
    assert!(outcome_custom.healed.is_none());
    assert!(outcome_custom.healing_attempts.is_none());
    assert!(outcome_custom.lesson_learned.is_none());

    // Branch B: Default triggerType (None -> "manual")
    let node_default = make_node(
        "trig_default",
        "triggerNode",
        serde_json::json!({}),
    );
    let outcome_default = execute_trigger_node(&node_default);
    assert_eq!(outcome_default.status, "completed");
    assert_eq!(outcome_default.node_output["triggered"], true);
    assert_eq!(outcome_default.node_output["type"], "manual");
    assert!(chrono::DateTime::parse_from_rfc3339(
        outcome_default.node_output["timestamp"].as_str().unwrap()
    ).is_ok());
    assert!(outcome_default.node_error.is_none());
}

#[test]
fn test_execute_approval_node_branches() {
    // Branch A: With required_role
    let node_role = make_node(
        "appr_1",
        "approvalNode",
        serde_json::json!({
            "requiredRole": "security_admin"
        }),
    );
    let outcome_role = execute_approval_node(&node_role);
    assert_eq!(outcome_role.status, "waiting_approval");
    assert_eq!(outcome_role.node_output["decision"], "Pending");
    assert_eq!(outcome_role.node_output["required_role"], "security_admin");
    assert!(outcome_role.node_error.is_none());
    assert!(outcome_role.engine_used.is_none());
    assert!(outcome_role.healed.is_none());
    assert!(outcome_role.healing_attempts.is_none());
    assert!(outcome_role.lesson_learned.is_none());

    // Branch B: Without required_role (None -> null)
    let node_no_role = make_node(
        "appr_2",
        "approvalNode",
        serde_json::json!({}),
    );
    let outcome_no_role = execute_approval_node(&node_no_role);
    assert_eq!(outcome_no_role.status, "waiting_approval");
    assert_eq!(outcome_no_role.node_output["decision"], "Pending");
    assert!(outcome_no_role.node_output["required_role"].is_null());
    assert!(outcome_no_role.node_error.is_none());
}

#[test]
fn test_execute_router_node_branches() {
    // Branch A: With expression
    let node_expr = make_node(
        "route_1",
        "routerNode",
        serde_json::json!({
            "expression": "context.score >= 90"
        }),
    );
    let outcome_expr = execute_router_node(&node_expr);
    assert_eq!(outcome_expr.status, "completed");
    assert_eq!(outcome_expr.node_output["route_selected"], "branch-a");
    assert_eq!(outcome_expr.node_output["condition"], "context.score >= 90");
    assert!(outcome_expr.node_error.is_none());
    assert!(outcome_expr.engine_used.is_none());
    assert!(outcome_expr.healed.is_none());
    assert!(outcome_expr.healing_attempts.is_none());
    assert!(outcome_expr.lesson_learned.is_none());

    // Branch B: Without expression (None -> "")
    let node_no_expr = make_node(
        "route_2",
        "routerNode",
        serde_json::json!({}),
    );
    let outcome_no_expr = execute_router_node(&node_no_expr);
    assert_eq!(outcome_no_expr.status, "completed");
    assert_eq!(outcome_no_expr.node_output["route_selected"], "branch-a");
    assert_eq!(outcome_no_expr.node_output["condition"], "");
    assert!(outcome_no_expr.node_error.is_none());
}

#[tokio::test]
async fn test_execute_tool_node_none_command() {
    let ws = TestWorkspace::new();
    let node = make_node("tool_none", "toolNode", serde_json::json!({}));
    let mut timeline = Vec::new();

    let outcome = execute_tool_node(
        ws.dir.to_str().unwrap(),
        &ws.orchestrator.harness_memory,
        &node,
        &node.id,
        "Tool Label",
        &mut timeline,
    )
    .await;

    assert_eq!(outcome.status, "completed");
    assert_eq!(outcome.node_output, serde_json::json!({}));
    assert!(outcome.node_error.is_none());
    assert!(outcome.engine_used.is_none());
    assert!(outcome.healed.is_none());
    assert!(outcome.healing_attempts.is_none());
    assert!(outcome.lesson_learned.is_none());
    assert!(timeline.is_empty());
}

#[tokio::test]
async fn test_execute_tool_node_command_success_json_and_plain_text() {
    let ws = TestWorkspace::new();

    // Branch A: Command returning valid JSON
    let node_json = make_node(
        "tool_json",
        "toolNode",
        serde_json::json!({
            "command": "echo '{\"status\": \"ok\", \"items\": [1, 2, 3]}'"
        }),
    );
    let mut timeline = Vec::new();
    let outcome_json = execute_tool_node(
        ws.dir.to_str().unwrap(),
        &ws.orchestrator.harness_memory,
        &node_json,
        &node_json.id,
        "JSON Tool",
        &mut timeline,
    )
    .await;

    assert_eq!(outcome_json.status, "completed");
    assert!(outcome_json.node_error.is_none());
    assert_eq!(outcome_json.node_output["status"], 0);
    assert_eq!(outcome_json.node_output["data"]["status"], "ok");
    assert_eq!(outcome_json.node_output["data"]["items"].as_array().unwrap().len(), 3);
    assert!(outcome_json.healed.is_none());

    // Branch B: Command returning plain text
    let node_text = make_node(
        "tool_text",
        "toolNode",
        serde_json::json!({
            "command": "echo 'plain text result'"
        }),
    );
    let mut timeline_text = Vec::new();
    let outcome_text = execute_tool_node(
        ws.dir.to_str().unwrap(),
        &ws.orchestrator.harness_memory,
        &node_text,
        &node_text.id,
        "Text Tool",
        &mut timeline_text,
    )
    .await;

    assert_eq!(outcome_text.status, "completed");
    assert!(outcome_text.node_error.is_none());
    assert_eq!(outcome_text.node_output["status"], 0);
    assert_eq!(outcome_text.node_output["stdout"], "plain text result");
    assert_eq!(outcome_text.node_output["data"]["raw"], "plain text result");
}

#[tokio::test]
async fn test_execute_tool_node_guardrail_safety_check_blocks_command() {
    let ws = TestWorkspace::new();

    // Dangerous command violating GuardrailPolicy
    let node_unsafe = make_node(
        "tool_unsafe",
        "toolNode",
        serde_json::json!({
            "command": "rm -rf /tmp/dangerous_dir"
        }),
    );
    let mut timeline = Vec::new();
    let outcome = execute_tool_node(
        ws.dir.to_str().unwrap(),
        &ws.orchestrator.harness_memory,
        &node_unsafe,
        &node_unsafe.id,
        "Unsafe Tool",
        &mut timeline,
    )
    .await;

    assert_eq!(outcome.status, "waiting_approval");
    assert!(outcome.node_error.is_some());
    assert!(outcome.node_error.unwrap().contains("Recursive deletion"));
    assert_eq!(outcome.node_output, serde_json::json!({}));
    assert!(outcome.healed.is_none());
}

#[tokio::test]
async fn test_execute_tool_node_failure_without_self_healing() {
    let ws = TestWorkspace::new();

    let node_fail = make_node(
        "tool_fail",
        "toolNode",
        serde_json::json!({
            "command": "sh -c 'echo \"Fatal script crash\" >&2; exit 5'",
            "enableSelfHealing": false
        }),
    );
    let mut timeline = Vec::new();
    let outcome = execute_tool_node(
        ws.dir.to_str().unwrap(),
        &ws.orchestrator.harness_memory,
        &node_fail,
        &node_fail.id,
        "Failing Tool",
        &mut timeline,
    )
    .await;

    assert_eq!(outcome.status, "failed");
    assert!(outcome.node_error.is_some());
    let err_str = outcome.node_error.unwrap();
    assert!(err_str.contains("Exit code 5: Fatal script crash"));
    assert_eq!(outcome.node_output["status"], 5);
    assert_eq!(outcome.node_output["stderr"], "Fatal script crash");
    assert!(outcome.healed.is_none());
    assert!(outcome.healing_attempts.is_none());
    assert!(outcome.lesson_learned.is_none());
}

#[tokio::test]
async fn test_execute_tool_node_self_healing_success_and_failure_after_attempt() {
    let ws = TestWorkspace::new();

    // 1. Self healing succeeds on retry
    let healed_target_dir = ws.dir.join("healed_target_dir");
    let target_str = healed_target_dir.to_str().unwrap();
    let heal_cmd = format!(
        "test -d {} || {{ echo 'directory not found' >&2; exit 1; }} # --output-dir {}",
        target_str, target_str
    );

    let node_heal = make_node(
        "tool_heal",
        "toolNode",
        serde_json::json!({
            "command": heal_cmd,
            "enableSelfHealing": true
        }),
    );
    let mut timeline = Vec::new();
    let outcome_heal = execute_tool_node(
        ws.dir.to_str().unwrap(),
        &ws.orchestrator.harness_memory,
        &node_heal,
        &node_heal.id,
        "Healable Tool",
        &mut timeline,
    )
    .await;

    assert_eq!(outcome_heal.status, "completed");
    assert!(outcome_heal.node_error.is_none());
    assert_eq!(outcome_heal.healed, Some(true));
    assert_eq!(outcome_heal.healing_attempts, Some(1));
    assert!(outcome_heal.lesson_learned.is_some());
    assert!(outcome_heal.lesson_learned.as_ref().unwrap().contains("Điều chỉnh lệnh thực thi sang:"));
    assert!(timeline.iter().any(|e| e.message.contains("⚡ [Harness² Self-Healing] Đã tự khắc phục lệnh Shell")));

    // 2. Self healing is attempted but still fails on retry (max_tool_attempts reached)
    let node_retry_fail = make_node(
        "tool_retry_fail",
        "toolNode",
        serde_json::json!({
            "command": "sh -c 'echo \"permission denied\" >&2; exit 127'",
            "enableSelfHealing": true
        }),
    );
    let mut timeline_fail = Vec::new();
    let outcome_retry_fail = execute_tool_node(
        ws.dir.to_str().unwrap(),
        &ws.orchestrator.harness_memory,
        &node_retry_fail,
        &node_retry_fail.id,
        "Unfixable Tool",
        &mut timeline_fail,
    )
    .await;

    assert_eq!(outcome_retry_fail.status, "failed");
    assert!(outcome_retry_fail.node_error.is_some());
    assert!(outcome_retry_fail.healed.is_none());
    assert!(timeline_fail.iter().any(|e| e.message.contains("🔄 [Harness² Propose-Probe] Tự động sửa lệnh Tool")));
}

#[tokio::test]
async fn test_execute_sub_workflow_node_branches() -> Result<(), Box<dyn std::error::Error>> {
    let ws = TestWorkspace::new();
    let parent_wf = test_workflow(
        serde_json::json!([
            {"id": "p_root", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([]),
    );

    // Branch 1: Missing subWorkflowId and endpoint
    let empty_sub_node = make_node(
        "sub_empty",
        "subWorkflowNode",
        serde_json::json!({}),
    );
    let context_state = HashMap::new();
    let mut last_agent_execution = None;
    let mut timeline = Vec::new();
    let mut ctx = NodeExecutionContext {
        context_state: &context_state,
        last_agent_execution: &mut last_agent_execution,
        depth: 0,
        timeline: &mut timeline,
    };
    let outcome_empty = execute_sub_workflow_node(
        &ws.orchestrator,
        &parent_wf,
        &empty_sub_node,
        &empty_sub_node.id,
        "Empty SubWF",
        &mut ctx,
    )
    .await;
    assert_eq!(outcome_empty.status, "failed");
    assert_eq!(
        outcome_empty.node_error,
        Some("Node Sub-Workflow chưa được gán subWorkflowId hợp lệ".to_string())
    );

    // Branch 2: Sub-workflow ID not found in catalog (catalog file missing)
    let missing_sub_node = make_node(
        "sub_missing",
        "subWorkflowNode",
        serde_json::json!({
            "subWorkflowId": "non-existent-subwf"
        }),
    );
    let outcome_missing = execute_sub_workflow_node(
        &ws.orchestrator,
        &parent_wf,
        &missing_sub_node,
        &missing_sub_node.id,
        "Missing SubWF",
        &mut ctx,
    )
    .await;
    assert_eq!(outcome_missing.status, "failed");
    assert!(outcome_missing
        .node_error
        .as_deref()
        .unwrap()
        .contains("Không thể nạp Sub-Workflow 'non-existent-subwf'"));

    // Set up catalog directory with child workflows
    let wf_dir = ws.dir.join("workflows");
    tokio::fs::create_dir_all(&wf_dir).await?;

    let child_completed = test_workflow(
        serde_json::json!([
            {"id": "c1", "type": "triggerNode", "data": {"triggerType": "sub-wf"}}
        ]),
        serde_json::json!([]),
    );
    let mut child_completed_def = child_completed;
    child_completed_def.id = "child-completed-id".to_string();
    child_completed_def.name = "Child Completed".to_string();

    let child_failed = test_workflow(
        serde_json::json!([
            {"id": "c_fail", "type": "unsupportedType", "data": {}}
        ]),
        serde_json::json!([]),
    );
    let mut child_failed_def = child_failed;
    child_failed_def.id = "child-failed-id".to_string();
    child_failed_def.name = "Child Failed".to_string();

    let catalog_data = serde_json::to_string_pretty(&vec![
        child_completed_def,
        child_failed_def,
    ])?;
    tokio::fs::write(wf_dir.join("catalog.json"), catalog_data).await?;

    // Branch 3: Sub-workflow execution succeeds via endpoint fallback
    let endpoint_sub_node = make_node(
        "sub_endpoint",
        "subWorkflowNode",
        serde_json::json!({
            "endpoint": "child-completed-id"
        }),
    );
    let outcome_success = execute_sub_workflow_node(
        &ws.orchestrator,
        &parent_wf,
        &endpoint_sub_node,
        &endpoint_sub_node.id,
        "Endpoint SubWF",
        &mut ctx,
    )
    .await;
    assert_eq!(outcome_success.status, "completed");
    assert!(outcome_success.node_error.is_none());
    assert_eq!(outcome_success.node_output["sub_workflow_id"], "child-completed-id");
    assert_eq!(outcome_success.node_output["sub_workflow_name"], "Child Completed");
    assert_eq!(outcome_success.node_output["status"], "Completed");
    assert_eq!(outcome_success.node_output["nodes_executed"], 1);
    assert!(ctx.timeline.iter().any(|e| e.message.contains("🔀 Đang lồng ghép Sub-Workflow: 'child-completed-id'")));
    assert!(ctx.timeline.iter().any(|e| e.message.contains("[SubWF: Child Completed]")));

    // Branch 4: Sub-workflow execution completes with non-success status ("Failed")
    let failing_sub_node = make_node(
        "sub_failing",
        "subWorkflowNode",
        serde_json::json!({
            "subWorkflowId": "child-failed-id"
        }),
    );
    let outcome_child_fail = execute_sub_workflow_node(
        &ws.orchestrator,
        &parent_wf,
        &failing_sub_node,
        &failing_sub_node.id,
        "Failing SubWF",
        &mut ctx,
    )
    .await;
    assert_eq!(outcome_child_fail.status, "failed");
    assert!(outcome_child_fail
        .node_error
        .as_deref()
        .unwrap()
        .contains("Sub-Workflow 'Child Failed' kết thúc với trạng thái: Failed"));
    assert_eq!(outcome_child_fail.node_output["status"], "Failed");

    // Branch 5: Sub-workflow internal execution error (depth limit exceeded)
    let mut ctx_deep = NodeExecutionContext {
        context_state: &context_state,
        last_agent_execution: &mut last_agent_execution,
        depth: 5, // Next depth will be 6 > 5
        timeline: &mut timeline,
    };
    let outcome_deep = execute_sub_workflow_node(
        &ws.orchestrator,
        &parent_wf,
        &endpoint_sub_node,
        &endpoint_sub_node.id,
        "Deep SubWF",
        &mut ctx_deep,
    )
    .await;
    assert_eq!(outcome_deep.status, "failed");
    assert!(outcome_deep
        .node_error
        .as_deref()
        .unwrap()
        .contains("Lỗi thực thi Sub-Workflow 'child-completed-id'"));
    assert!(outcome_deep
        .node_error
        .as_deref()
        .unwrap()
        .contains("Vượt quá độ sâu tối đa"));

    Ok(())
}

#[tokio::test]
async fn test_execute_node_all_branches_and_fallback() -> Result<(), Box<dyn std::error::Error>> {
    let ws = TestWorkspace::new();
    let wf = test_workflow(serde_json::json!([]), serde_json::json!([]));
    let context_state = HashMap::new();
    let mut last_agent_execution = None;
    let mut timeline = Vec::new();
    let mut ctx = NodeExecutionContext {
        context_state: &context_state,
        last_agent_execution: &mut last_agent_execution,
        depth: 0,
        timeline: &mut timeline,
    };

    // 1. "triggerNode"
    let trigger_node = make_node("n_trig", "triggerNode", serde_json::json!({"triggerType": "webhook"}));
    let trig_res = execute_node(&ws.orchestrator, &wf, &trigger_node, &trigger_node.id, &mut ctx).await?;
    assert_eq!(trig_res.status, "completed");
    assert_eq!(trig_res.node_output["type"], "webhook");

    // 2. "approvalNode"
    let approval_node = make_node("n_appr", "approvalNode", serde_json::json!({"requiredRole": "lead"}));
    let appr_res = execute_node(&ws.orchestrator, &wf, &approval_node, &approval_node.id, &mut ctx).await?;
    assert_eq!(appr_res.status, "waiting_approval");
    assert_eq!(appr_res.node_output["required_role"], "lead");

    // 3. "routerNode"
    let router_node = make_node("n_route", "routerNode", serde_json::json!({"expression": "a == b"}));
    let route_res = execute_node(&ws.orchestrator, &wf, &router_node, &router_node.id, &mut ctx).await?;
    assert_eq!(route_res.status, "completed");
    assert_eq!(route_res.node_output["condition"], "a == b");

    // 4. "toolNode"
    let tool_node = make_node("n_tool", "toolNode", serde_json::json!({"command": "echo test_tool_node"}));
    let tool_res = execute_node(&ws.orchestrator, &wf, &tool_node, &tool_node.id, &mut ctx).await?;
    assert_eq!(tool_res.status, "completed");
    assert_eq!(tool_res.node_output["stdout"], "test_tool_node");

    // Set up catalog for subWorkflowNode and workflowNode
    let wf_dir = ws.dir.join("workflows");
    tokio::fs::create_dir_all(&wf_dir).await?;
    let mut sub_wf = test_workflow(
        serde_json::json!([
            {"id": "sub_t", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([]),
    );
    sub_wf.id = "catalog-sub-wf".to_string();
    sub_wf.name = "Catalog Sub WF".to_string();
    tokio::fs::write(wf_dir.join("catalog.json"), serde_json::to_string(&vec![sub_wf])?).await?;

    // 5. "subWorkflowNode"
    let sub_wf_node = make_node(
        "n_sub",
        "subWorkflowNode",
        serde_json::json!({"subWorkflowId": "catalog-sub-wf", "label": "Custom SubWF Label"}),
    );
    let sub_wf_res = execute_node(&ws.orchestrator, &wf, &sub_wf_node, &sub_wf_node.id, &mut ctx).await?;
    assert_eq!(sub_wf_res.status, "completed");
    assert_eq!(sub_wf_res.node_output["sub_workflow_id"], "catalog-sub-wf");

    // 6. "workflowNode" (alias for subWorkflowNode)
    let alias_node = make_node(
        "n_alias",
        "workflowNode",
        serde_json::json!({"subWorkflowId": "catalog-sub-wf"}),
    );
    let alias_res = execute_node(&ws.orchestrator, &wf, &alias_node, &alias_node.id, &mut ctx).await?;
    assert_eq!(alias_res.status, "completed");
    assert_eq!(alias_res.node_output["sub_workflow_id"], "catalog-sub-wf");

    // 7. Fallback "_" for unsupported node type
    let unsupported_node = make_node(
        "n_unsupported",
        "futureCustomNodeType",
        serde_json::json!({"label": "Unsupported Label"}),
    );
    let fallback_res = execute_node(&ws.orchestrator, &wf, &unsupported_node, &unsupported_node.id, &mut ctx).await?;
    assert_eq!(fallback_res.status, "failed");
    assert_eq!(fallback_res.node_output, serde_json::json!({}));
    assert_eq!(
        fallback_res.node_error,
        Some("Unsupported node type: futureCustomNodeType".to_string())
    );
    assert!(fallback_res.engine_used.is_none());
    assert!(fallback_res.healed.is_none());
    assert!(fallback_res.healing_attempts.is_none());
    assert!(fallback_res.lesson_learned.is_none());

    // 8. Fallback "_" with missing label (defaults to current_id in node_label)
    let unsupported_no_label = make_node(
        "n_no_label_id",
        "anotherCustomNodeType",
        serde_json::json!({}),
    );
    let fallback_res2 = execute_node(&ws.orchestrator, &wf, &unsupported_no_label, "n_no_label_id", &mut ctx).await?;
    assert_eq!(fallback_res2.status, "failed");
    assert_eq!(
        fallback_res2.node_error,
        Some("Unsupported node type: anotherCustomNodeType".to_string())
    );

    Ok(())
}


