use agent_runtime::workflow_dag::{
    WorkflowDag, WorkflowDefinition, WorkflowEdge, WorkflowNode, WorkflowNodeData,
};

fn create_sample_dag() -> WorkflowDag {
    let node1 = WorkflowNode {
        id: "start".to_string(),
        node_type: "triggerNode".to_string(),
        data: WorkflowNodeData {
            label: Some("Start Trigger".to_string()),
            ..Default::default()
        },
    };
    let node2 = WorkflowNode {
        id: "process".to_string(),
        node_type: "agentNode".to_string(),
        data: WorkflowNodeData {
            label: Some("Agent Processor".to_string()),
            persona_name: Some("Coder".to_string()),
            ..Default::default()
        },
    };
    let node3 = WorkflowNode {
        id: "end".to_string(),
        node_type: "approvalNode".to_string(),
        data: WorkflowNodeData {
            label: None,
            ..Default::default()
        },
    };

    let edge1 = WorkflowEdge {
        id: "e1".to_string(),
        source: "start".to_string(),
        target: "process".to_string(),
        source_handle: None,
        label: Some("on_trigger".to_string()),
    };
    let edge2 = WorkflowEdge {
        id: "e2".to_string(),
        source: "process".to_string(),
        target: "end".to_string(),
        source_handle: None,
        label: None,
    };

    WorkflowDefinition {
        id: "test_pipeline".to_string(),
        name: "Test Pipeline".to_string(),
        description: Some("Visualization pipeline test".to_string()),
        version: Some("1.0.0".to_string()),
        nodes: vec![node1, node2, node3],
        edges: vec![edge1, edge2],
    }
}

#[test]
fn test_workflow_dag_to_mermaid() {
    let dag = create_sample_dag();
    let mermaid = dag.to_mermaid();

    // Verify Mermaid graph header
    assert!(
        mermaid.starts_with("graph TD"),
        "Mermaid output should start with 'graph TD', got:\n{}",
        mermaid
    );

    // Verify nodes are rendered with labels or IDs
    assert!(
        mermaid.contains("start[\"Start Trigger\"]") || mermaid.contains("start[Start Trigger]")
    );
    assert!(
        mermaid.contains("process[\"Agent Processor\"]")
            || mermaid.contains("process[Agent Processor]")
    );
    assert!(mermaid.contains("end"));

    // Verify edges are rendered properly with and without labels
    assert!(
        mermaid.contains("start -->|on_trigger| process")
            || mermaid.contains("start -- \"on_trigger\" --> process")
            || mermaid.contains("start -->|\"on_trigger\"| process")
    );
    assert!(mermaid.contains("process --> end"));
}

#[test]
fn test_workflow_dag_to_dot() {
    let dag = create_sample_dag();
    let dot = dag.to_dot();

    // Verify Graphviz DOT digraph structure
    assert!(
        dot.starts_with("digraph"),
        "DOT output should start with 'digraph', got:\n{}",
        dot
    );
    assert!(dot.contains('{') && dot.ends_with('}'));

    // Verify nodes exist in DOT
    assert!(dot.contains("start"));
    assert!(dot.contains("process"));
    assert!(dot.contains("end"));

    // Verify edges with and without labels
    assert!(dot.contains("start -> process"));
    assert!(dot.contains("label=\"on_trigger\"") || dot.contains("label = \"on_trigger\""));
    assert!(dot.contains("process -> end"));
}
