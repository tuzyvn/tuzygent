pub mod agent;
pub mod control_flow;
pub mod sub_workflow;
pub mod tool;
pub mod types;

pub use agent::execute_agent_node;
pub use control_flow::{
    execute_approval_node, execute_consensus_gate_node, execute_router_node, execute_trigger_node,
};
pub use sub_workflow::execute_sub_workflow_node;
pub use tool::execute_tool_node;
pub use types::{NodeExecutionContext, NodeExecutionOutcome};

use anyhow::Result;

use super::orchestrator::WorkflowOrchestrator;
use super::types::{WorkflowDefinition, WorkflowNode};

pub async fn execute_node(
    orchestrator: &WorkflowOrchestrator,
    workflow: &WorkflowDefinition,
    node: &WorkflowNode,
    current_id: &str,
    ctx: &mut NodeExecutionContext<'_>,
) -> Result<NodeExecutionOutcome> {
    let node_label = node
        .data
        .label
        .clone()
        .unwrap_or_else(|| current_id.to_string());

    match node.node_type.as_str() {
        "triggerNode" => Ok(execute_trigger_node(node)),
        "agentNode" => execute_agent_node(orchestrator, node, &node_label, ctx).await,
        "toolNode" => Ok(execute_tool_node(
            &orchestrator.workspace_dir,
            &orchestrator.harness_memory,
            node,
            current_id,
            &node_label,
            ctx.timeline,
        )
        .await),
        "approvalNode" => Ok(execute_approval_node(node)),
        "routerNode" => Ok(execute_router_node(node)),
        "consensusGateNode" | "consensusGate" | "ConsensusGate" => {
            Ok(execute_consensus_gate_node(node, ctx.context_state))
        }
        "subWorkflowNode" | "workflowNode" => Ok(execute_sub_workflow_node(
            orchestrator,
            workflow,
            node,
            current_id,
            &node_label,
            ctx,
        )
        .await),
        _ => Ok(NodeExecutionOutcome {
            node_output: serde_json::json!({}),
            node_error: Some(format!("Unsupported node type: {}", node.node_type)),
            engine_used: None,
            status: "failed".to_string(),
            healed: None,
            healing_attempts: None,
            lesson_learned: None,
        }),
    }
}
