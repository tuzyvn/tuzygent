use chrono::Utc;

use super::types::{NodeExecutionContext, NodeExecutionOutcome};
use crate::workflow_dag::orchestrator::WorkflowOrchestrator;
use crate::workflow_dag::types::{WorkflowDefinition, WorkflowNode, WorkflowTimelineEvent};

pub async fn execute_sub_workflow_node(
    orchestrator: &WorkflowOrchestrator,
    workflow: &WorkflowDefinition,
    node: &WorkflowNode,
    current_id: &str,
    node_label: &str,
    ctx: &mut NodeExecutionContext<'_>,
) -> NodeExecutionOutcome {
    let sub_id = node
        .data
        .sub_workflow_id
        .as_deref()
        .or(node.data.endpoint.as_deref())
        .unwrap_or("");

    let mut node_output = serde_json::json!({});
    let mut node_error: Option<String> = None;
    let status;

    if sub_id.is_empty() {
        status = "failed".to_string();
        node_error = Some("Node Sub-Workflow chưa được gán subWorkflowId hợp lệ".to_string());
    } else {
        ctx.timeline.push(WorkflowTimelineEvent {
            timestamp: Utc::now().format("%H:%M:%S").to_string(),
            level: "info".to_string(),
            message: format!("🔀 Đang lồng ghép Sub-Workflow: '{}' (Độ sâu: {})", sub_id, ctx.depth + 1),
        });

        match orchestrator.load_workflow_from_catalog(sub_id).await {
            Ok(child_wf) => {
                let child_payload = Some(serde_json::json!({
                    "parent_context": ctx.context_state.clone(),
                    "parent_workflow_id": workflow.id.clone(),
                    "parent_node_id": current_id.to_string()
                }));

                let caller_desc = format!("Parent Workflow '{}' (Node: {})", workflow.name, node_label);
                match orchestrator
                    .execute_workflow_internal(&child_wf, child_payload, &caller_desc, ctx.depth + 1)
                    .await
                {
                    Ok(child_report) => {
                        for event in &child_report.timeline {
                            ctx.timeline.push(WorkflowTimelineEvent {
                                timestamp: event.timestamp.clone(),
                                level: event.level.clone(),
                                message: format!("[SubWF: {}] {}", child_wf.name, event.message),
                            });
                        }

                        let is_success = child_report.status.eq_ignore_ascii_case("completed");
                        status = if is_success {
                            "completed".to_string()
                        } else {
                            "failed".to_string()
                        };
                        if !is_success {
                            node_error = Some(format!(
                                "Sub-Workflow '{}' kết thúc với trạng thái: {}",
                                child_wf.name, child_report.status
                            ));
                        }

                        node_output = serde_json::json!({
                            "sub_workflow_id": child_wf.id,
                            "sub_workflow_name": child_wf.name,
                            "sub_run_id": child_report.run_id,
                            "status": child_report.status,
                            "duration_ms": child_report.duration_ms,
                            "nodes_executed": child_report.node_results.len(),
                            "node_results": child_report.node_results
                        });
                    }
                    Err(e) => {
                        status = "failed".to_string();
                        node_error = Some(format!("Lỗi thực thi Sub-Workflow '{}': {}", sub_id, e));
                    }
                }
            }
            Err(e) => {
                status = "failed".to_string();
                node_error = Some(format!("Không thể nạp Sub-Workflow '{}': {}", sub_id, e));
            }
        }
    }

    NodeExecutionOutcome {
        node_output,
        node_error,
        engine_used: None,
        status,
        healed: None,
        healing_attempts: None,
        lesson_learned: None,
    }
}
