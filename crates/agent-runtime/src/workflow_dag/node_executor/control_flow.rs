use chrono::Utc;

use super::types::NodeExecutionOutcome;
use crate::workflow_dag::types::WorkflowNode;

pub fn execute_trigger_node(node: &WorkflowNode) -> NodeExecutionOutcome {
    let node_output = serde_json::json!({
        "triggered": true,
        "type": node.data.trigger_type.clone().unwrap_or_else(|| "manual".to_string()),
        "timestamp": Utc::now().to_rfc3339()
    });
    NodeExecutionOutcome {
        node_output,
        node_error: None,
        engine_used: None,
        status: "completed".to_string(),
        healed: None,
        healing_attempts: None,
        lesson_learned: None,
    }
}

pub fn execute_approval_node(node: &WorkflowNode) -> NodeExecutionOutcome {
    let node_output = serde_json::json!({
        "decision": "Pending",
        "required_role": node.data.required_role,
    });
    NodeExecutionOutcome {
        node_output,
        node_error: None,
        engine_used: None,
        status: "waiting_approval".to_string(),
        healed: None,
        healing_attempts: None,
        lesson_learned: None,
    }
}

pub fn execute_router_node(node: &WorkflowNode) -> NodeExecutionOutcome {
    let node_output = serde_json::json!({
        "route_selected": "branch-a",
        "condition": node.data.expression.clone().unwrap_or_default()
    });
    NodeExecutionOutcome {
        node_output,
        node_error: None,
        engine_used: None,
        status: "completed".to_string(),
        healed: None,
        healing_attempts: None,
        lesson_learned: None,
    }
}
