use super::test_workflow;
use crate::workflow_dag::*;

#[test]
fn test_dag_validation_edge_pointing_to_non_existent_target_returns_err_without_panic() {
    let wf = test_workflow(
        serde_json::json!([
            {"id": "node_a", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "edge_1", "source": "node_a", "target": "missing_target"}
        ]),
    );

    let result = std::panic::catch_unwind(|| validate_workflow(&wf));
    let validation_res = result.expect("validate_workflow should not panic on missing edge target");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err when edge targets non-existent node"
    );
    let err_msg = validation_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("references an unknown node"),
        "Error message should mention unknown node: {}",
        err_msg
    );
}

#[test]
fn test_dag_validation_edge_pointing_from_non_existent_source_returns_err_without_panic() {
    let wf = test_workflow(
        serde_json::json!([
            {"id": "node_b", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "edge_2", "source": "missing_source", "target": "node_b"}
        ]),
    );

    let result = std::panic::catch_unwind(|| validate_workflow(&wf));
    let validation_res = result.expect("validate_workflow should not panic on missing edge source");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err when edge source does not exist"
    );
    let err_msg = validation_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("references an unknown node"),
        "Error message should mention unknown node: {}",
        err_msg
    );
}

#[test]
fn test_dag_validation_edge_with_both_source_and_target_missing_returns_err_without_panic() {
    let wf = test_workflow(
        serde_json::json!([
            {"id": "node_isolated", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "edge_ghost", "source": "phantom_source", "target": "phantom_target"}
        ]),
    );

    let result = std::panic::catch_unwind(|| validate_workflow(&wf));
    let validation_res = result.expect("validate_workflow should not panic when both endpoints are missing");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err when both endpoints are missing"
    );
}

#[test]
fn test_dag_validation_empty_nodes_list_returns_err_without_panic() {
    let wf = test_workflow(serde_json::json!([]), serde_json::json!([]));

    let result = std::panic::catch_unwind(|| validate_workflow(&wf));
    let validation_res = result.expect("validate_workflow should not panic on empty nodes list");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err when nodes list is empty"
    );
    let err_msg = validation_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("Workflow must contain at least one node"),
        "Error should state workflow must contain at least one node: {}",
        err_msg
    );
}

#[test]
fn test_dag_validation_empty_or_whitespace_node_id_returns_err_without_panic() {
    let empty_id_wf = test_workflow(
        serde_json::json!([
            {"id": "", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([]),
    );

    let result_empty = std::panic::catch_unwind(|| validate_workflow(&empty_id_wf));
    let validation_res_empty = result_empty.expect("validate_workflow should not panic on empty node id");
    assert!(
        validation_res_empty.is_err(),
        "Expected Result::Err for empty node id"
    );
    let err_msg = validation_res_empty.unwrap_err().to_string();
    assert!(
        err_msg.contains("Empty or duplicate node ID"),
        "Unexpected error message: {}",
        err_msg
    );

    let whitespace_id_wf = test_workflow(
        serde_json::json!([
            {"id": "   ", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([]),
    );

    let result_ws = std::panic::catch_unwind(|| validate_workflow(&whitespace_id_wf));
    let validation_res_ws = result_ws.expect("validate_workflow should not panic on whitespace node id");
    assert!(
        validation_res_ws.is_err(),
        "Expected Result::Err for whitespace-only node id"
    );
}

#[test]
fn test_dag_validation_edge_pointing_to_or_from_empty_node_id_returns_err_without_panic() {
    let edge_to_empty_wf = test_workflow(
        serde_json::json!([
            {"id": "node_valid", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "edge_empty_target", "source": "node_valid", "target": ""}
        ]),
    );

    let result = std::panic::catch_unwind(|| validate_workflow(&edge_to_empty_wf));
    let validation_res = result.expect("validate_workflow should not panic on edge with empty target");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err when edge targets empty string ID"
    );

    let edge_from_empty_wf = test_workflow(
        serde_json::json!([
            {"id": "node_valid", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "edge_empty_source", "source": "", "target": "node_valid"}
        ]),
    );

    let result = std::panic::catch_unwind(|| validate_workflow(&edge_from_empty_wf));
    let validation_res = result.expect("validate_workflow should not panic on edge with empty source");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err when edge sources from empty string ID"
    );
}

#[test]
fn test_dag_validation_duplicate_node_ids_returns_err_without_panic() {
    let wf = test_workflow(
        serde_json::json!([
            {"id": "node_dup", "type": "triggerNode", "data": {}},
            {"id": "node_dup", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([]),
    );

    let result = std::panic::catch_unwind(|| validate_workflow(&wf));
    let validation_res = result.expect("validate_workflow should not panic on duplicate node id");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err for duplicate node id"
    );
    let err_msg = validation_res.unwrap_err().to_string();
    assert!(err_msg.contains("Empty or duplicate node ID"));
}

#[test]
fn test_dag_validation_cycle_returns_err_without_panic() {
    let wf = test_workflow(
        serde_json::json!([
            {"id": "n1", "type": "triggerNode", "data": {}},
            {"id": "n2", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "e1", "source": "n1", "target": "n2"},
            {"id": "e2", "source": "n2", "target": "n1"}
        ]),
    );

    let result = std::panic::catch_unwind(|| validate_workflow(&wf));
    let validation_res = result.expect("validate_workflow should not panic on cyclic graph");
    assert!(
        validation_res.is_err(),
        "Expected Result::Err for cyclic workflow"
    );
    let err_msg = validation_res.unwrap_err().to_string();
    assert!(err_msg.contains("Workflow contains a cycle"));
}


#[test]
fn test_dag_validation_edge_cases_safe_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Self-loop cycle (node pointing to itself)
    let self_loop_wf = test_workflow(
        serde_json::json!([
            {"id": "node_self", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "edge_self", "source": "node_self", "target": "node_self"}
        ]),
    );
    let self_loop_res = std::panic::catch_unwind(|| validate_workflow(&self_loop_wf));
    let self_loop_res = match self_loop_res {
        Ok(res) => res,
        Err(_) => return Err("validate_workflow panicked on self-loop cycle".into()),
    };
    match self_loop_res {
        Ok(_) => return Err("Expected self-loop workflow to fail validation".into()),
        Err(err) => {
            if !err.to_string().contains("cycle") {
                return Err(format!("Expected cycle error, got: {}", err).into());
            }
        }
    }

    // 2. Disconnected component with a cycle
    let disconnected_cycle_wf = test_workflow(
        serde_json::json!([
            {"id": "node_isolated", "type": "triggerNode", "data": {}},
            {"id": "cycle_a", "type": "customNode", "data": {}},
            {"id": "cycle_b", "type": "customNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "e_ab", "source": "cycle_a", "target": "cycle_b"},
            {"id": "e_ba", "source": "cycle_b", "target": "cycle_a"}
        ]),
    );
    let disconnected_res = std::panic::catch_unwind(|| validate_workflow(&disconnected_cycle_wf));
    let disconnected_res = match disconnected_res {
        Ok(res) => res,
        Err(_) => {
            return Err("validate_workflow panicked on disconnected component with cycle".into())
        }
    };
    match disconnected_res {
        Ok(_) => return Err("Expected disconnected cycle workflow to fail validation".into()),
        Err(err) => {
            if !err.to_string().contains("cycle") {
                return Err(format!("Expected cycle error, got: {}", err).into());
            }
        }
    }

    // 3. Edge with whitespace-only endpoints pointing to unknown node
    let ws_edge_wf = test_workflow(
        serde_json::json!([
            {"id": "node_valid", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "e_ws", "source": "   ", "target": "node_valid"}
        ]),
    );
    let ws_edge_res = std::panic::catch_unwind(|| validate_workflow(&ws_edge_wf));
    let ws_edge_res = match ws_edge_res {
        Ok(res) => res,
        Err(_) => return Err("validate_workflow panicked on whitespace edge endpoint".into()),
    };
    match ws_edge_res {
        Ok(_) => return Err("Expected whitespace edge to fail validation".into()),
        Err(err) => {
            if !err.to_string().contains("references an unknown node") {
                return Err(format!("Expected unknown node error, got: {}", err).into());
            }
        }
    }

    // 4. Valid DAG with diamond dependencies propagates Ok(()) cleanly
    let diamond_wf = test_workflow(
        serde_json::json!([
            {"id": "root", "type": "triggerNode", "data": {}},
            {"id": "branch_a", "type": "customNode", "data": {}},
            {"id": "branch_b", "type": "customNode", "data": {}},
            {"id": "join", "type": "customNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "e1", "source": "root", "target": "branch_a"},
            {"id": "e2", "source": "root", "target": "branch_b"},
            {"id": "e3", "source": "branch_a", "target": "join"},
            {"id": "e4", "source": "branch_b", "target": "join"}
        ]),
    );
    validate_workflow(&diamond_wf)?;

    Ok(())
}

