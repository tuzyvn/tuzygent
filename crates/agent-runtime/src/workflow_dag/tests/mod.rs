use super::*;

pub(crate) fn test_workflow(nodes: serde_json::Value, edges: serde_json::Value) -> WorkflowDefinition {
    serde_json::from_value(serde_json::json!({
        "id": "wf-test",
        "name": "Validation Test Workflow",
        "nodes": nodes,
        "edges": edges,
    }))
    .expect("Workflow JSON must deserialize into WorkflowDefinition")
}

mod node_executor_tests;
mod orchestrator_tests;
mod validation_tests;
