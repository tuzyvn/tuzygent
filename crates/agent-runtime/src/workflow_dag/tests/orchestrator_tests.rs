use super::test_workflow;
use crate::workflow_dag::*;

#[tokio::test]
async fn test_orchestrator_execute_workflow_boundary_cases_return_err_without_panic(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = std::env::temp_dir().join(format!("tuzygent-wf-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    let dir_str = tmp_dir
        .to_str()
        .ok_or("Failed to convert temp directory path to string")?;
    let orchestrator = WorkflowOrchestrator::new(dir_str, dir_str);

    // Boundary case 1: Edge targets non-existent node
    let wf_missing_target = test_workflow(
        serde_json::json!([
            {"id": "root", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "e_bad", "source": "root", "target": "ghost_node"}
        ]),
    );
    let res = orchestrator.execute_workflow(&wf_missing_target, None, "test").await;
    assert!(res.is_err(), "execute_workflow must return Err for missing edge target");

    // Boundary case 2: Edge sources from non-existent node
    let wf_missing_source = test_workflow(
        serde_json::json!([
            {"id": "root", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "e_bad", "source": "ghost_node", "target": "root"}
        ]),
    );
    let res = orchestrator.execute_workflow(&wf_missing_source, None, "test").await;
    assert!(res.is_err(), "execute_workflow must return Err for missing edge source");

    // Boundary case 3: Empty nodes list
    let wf_empty_nodes = test_workflow(serde_json::json!([]), serde_json::json!([]));
    let res = orchestrator.execute_workflow(&wf_empty_nodes, None, "test").await;
    assert!(res.is_err(), "execute_workflow must return Err for empty nodes list");

    // Boundary case 4: Node with empty ID
    let wf_empty_node_id = test_workflow(
        serde_json::json!([
            {"id": "", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([]),
    );
    let res = orchestrator.execute_workflow(&wf_empty_node_id, None, "test").await;
    assert!(res.is_err(), "execute_workflow must return Err for empty node id");

    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}

#[tokio::test]
async fn test_orchestrator_boundary_errors_and_result_propagation(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir =
        std::env::temp_dir().join(format!("tuzygent-wf-edge-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    let dir_str = tmp_dir
        .to_str()
        .ok_or("Failed to convert temp path to str")?;
    let orchestrator = WorkflowOrchestrator::new(dir_str, dir_str);

    // 1. Boundary: Sub-workflow recursion depth exceeding 5
    let valid_wf = test_workflow(
        serde_json::json!([
            {"id": "node_root", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([]),
    );
    let depth_err = orchestrator
        .execute_workflow_internal(&valid_wf, None, "test", 6)
        .await;
    match depth_err {
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err("Expected depth > 5 to return Err".into());
        }
        Err(err) => {
            if !err.to_string().contains("Vượt quá độ sâu tối đa") {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(format!("Unexpected depth error message: {}", err).into());
            }
        }
    }

    // 2. Boundary: Loading non-existent catalog file fails safely
    let catalog_err = orchestrator.load_workflow_from_catalog("missing-wf-id").await;
    match catalog_err {
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err("Expected missing catalog to return Err".into());
        }
        Err(err) => {
            if !err.to_string().contains("Không đọc được file catalog") {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(format!("Unexpected catalog error message: {}", err).into());
            }
        }
    }

    // 3. Boundary: Empty catalog JSON file where workflow ID is not found
    let wf_dir = tmp_dir.join("workflows");
    tokio::fs::create_dir_all(&wf_dir).await?;
    tokio::fs::write(wf_dir.join("catalog.json"), "[]").await?;
    let catalog_empty_err = orchestrator
        .load_workflow_from_catalog("non-existent-id")
        .await;
    match catalog_empty_err {
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err("Expected missing workflow ID in catalog to return Err".into());
        }
        Err(err) => {
            if !err.to_string().contains("Không tìm thấy Sub-Workflow") {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(format!("Unexpected catalog not found error message: {}", err).into());
            }
        }
    }

    // 4. Boundary: Malformed catalog JSON file returns Err safely
    tokio::fs::write(wf_dir.join("catalog.json"), "{invalid_json}").await?;
    let malformed_err = orchestrator.load_workflow_from_catalog("any-id").await;
    match malformed_err {
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err("Expected malformed catalog JSON to return Err".into());
        }
        Err(err) => {
            if !err.to_string().contains("Lỗi parse JSON catalog") {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(format!("Unexpected malformed catalog error message: {}", err).into());
            }
        }
    }

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}

#[tokio::test]
async fn test_orchestrator_executes_trigger_and_router_nodes_and_handles_unsupported_node() -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = std::env::temp_dir().join(format!("tuzygent-wf-dispatch-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    let dir_str = tmp_dir.to_str().ok_or("Failed to convert temp path to str")?;
    let orchestrator = WorkflowOrchestrator::new(dir_str, dir_str);

    // 1. Valid execution with triggerNode and routerNode
    let wf_valid = test_workflow(
        serde_json::json!([
            {"id": "t1", "type": "triggerNode", "data": {"triggerType": "webhook"}},
            {"id": "r1", "type": "routerNode", "data": {"expression": "score > 80"}}
        ]),
        serde_json::json!([
            {"id": "e_tr", "source": "t1", "target": "r1"}
        ]),
    );

    let report = orchestrator.execute_workflow(&wf_valid, Some(serde_json::json!({"req_id": 42})), "tester").await?;
    assert_eq!(report.status, "Completed");
    assert_eq!(report.node_results["t1"].status, "completed");
    assert_eq!(report.node_results["t1"].output["type"], "webhook");
    assert_eq!(report.node_results["r1"].status, "completed");
    assert_eq!(report.node_results["r1"].output["route_selected"], "branch-a");
    assert_eq!(report.node_results["r1"].output["condition"], "score > 80");

    // 2. Unsupported node type fails safely and skips dependents
    let wf_unsupported = test_workflow(
        serde_json::json!([
            {"id": "u1", "type": "unknownNodeType", "data": {}},
            {"id": "t2", "type": "triggerNode", "data": {}}
        ]),
        serde_json::json!([
            {"id": "e_ut", "source": "u1", "target": "t2"}
        ]),
    );

    let report_unsupported = orchestrator.execute_workflow(&wf_unsupported, None, "tester").await?;
    assert_eq!(report_unsupported.status, "Failed");
    assert_eq!(report_unsupported.node_results["u1"].status, "failed");
    assert!(report_unsupported.node_results["u1"].error.as_deref().is_some_and(|err| err.contains("Unsupported node type")));
    assert_eq!(report_unsupported.node_results["t2"].status, "skipped");

    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}

