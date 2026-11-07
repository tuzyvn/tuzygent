use super::test_workflow;
use crate::workflow_dag::*;
use std::collections::HashMap;

#[test]
fn test_consensus_gate_constructor_and_builder() {
    let gate = WorkflowNode::ConsensusGate(
        "gate_security",
        2,
        vec![
            "sec_reviewer_1".to_string(),
            "sec_reviewer_2".to_string(),
            "sec_reviewer_3".to_string(),
        ],
    )
    .with_label("Security Review Gate")
    .with_quorum_threshold(0.66);

    assert_eq!(gate.id, "gate_security");
    assert_eq!(gate.node_type, "consensusGateNode");
    assert_eq!(gate.data.label.as_deref(), Some("Security Review Gate"));
    assert_eq!(gate.data.required_votes, Some(2));
    assert_eq!(
        gate.data.reviewers,
        Some(vec![
            "sec_reviewer_1".to_string(),
            "sec_reviewer_2".to_string(),
            "sec_reviewer_3".to_string(),
        ])
    );
    assert_eq!(gate.data.quorum_threshold, Some(0.66));
}

#[test]
fn test_consensus_gate_sufficient_votes_scenario() {
    let gate = WorkflowNode::ConsensusGate(
        "gate_1",
        2,
        vec![
            "reviewer_a".to_string(),
            "reviewer_b".to_string(),
            "reviewer_c".to_string(),
        ],
    );

    let mut context_state = HashMap::new();
    context_state.insert(
        "reviewer_a".to_string(),
        serde_json::json!({ "vote": "approve", "notes": "LGTM" }),
    );
    context_state.insert(
        "reviewer_b".to_string(),
        serde_json::json!({ "decision": "Approved", "confidence": 0.95 }),
    );
    context_state.insert(
        "reviewer_c".to_string(),
        serde_json::json!({ "vote": "reject", "reason": "Requires more documentation" }),
    );

    let outcome = execute_consensus_gate_node(&gate, &context_state);

    assert_eq!(outcome.status, "completed");
    assert!(outcome.node_error.is_none());
    assert_eq!(outcome.node_output["consensus_reached"], true);
    assert_eq!(outcome.node_output["decision"], "Approved");
    assert_eq!(outcome.node_output["approved_votes"], 2);
    assert_eq!(outcome.node_output["rejected_votes"], 1);
    assert_eq!(outcome.node_output["missing_votes"], 0);
    assert_eq!(outcome.node_output["required_votes"], 2);
    assert_eq!(outcome.node_output["total_reviewers"], 3);
    assert_eq!(outcome.node_output["votes"]["reviewer_a"], "approved");
    assert_eq!(outcome.node_output["votes"]["reviewer_b"], "approved");
    assert_eq!(outcome.node_output["votes"]["reviewer_c"], "rejected");
}

#[test]
fn test_consensus_gate_insufficient_votes_scenario() {
    let gate = WorkflowNode::ConsensusGate(
        "gate_strict",
        3,
        vec![
            "reviewer_a".to_string(),
            "reviewer_b".to_string(),
            "reviewer_c".to_string(),
        ],
    );

    let mut context_state = HashMap::new();
    context_state.insert(
        "reviewer_a".to_string(),
        serde_json::json!({ "vote": "approve" }),
    );
    context_state.insert(
        "reviewer_b".to_string(),
        serde_json::json!({ "vote": "reject" }),
    );
    context_state.insert(
        "reviewer_c".to_string(),
        serde_json::json!({ "decision": "rejected" }),
    );

    let outcome = execute_consensus_gate_node(&gate, &context_state);

    assert_eq!(outcome.status, "failed");
    assert!(outcome.node_error.is_some());
    let err_msg = outcome.node_error.unwrap();
    assert!(err_msg.contains("Consensus not reached"));
    assert!(err_msg.contains("required 3 approved votes, received 1"));

    assert_eq!(outcome.node_output["consensus_reached"], false);
    assert_eq!(outcome.node_output["decision"], "Rejected");
    assert_eq!(outcome.node_output["approved_votes"], 1);
    assert_eq!(outcome.node_output["rejected_votes"], 2);
    assert_eq!(outcome.node_output["missing_votes"], 0);
    assert_eq!(outcome.node_output["required_votes"], 3);
}

#[test]
fn test_consensus_gate_missing_reviewers_insufficient_votes() {
    let gate = WorkflowNode::ConsensusGate(
        "gate_missing",
        2,
        vec![
            "reviewer_a".to_string(),
            "reviewer_b".to_string(),
            "reviewer_c".to_string(),
        ],
    );

    // Only reviewer_a has cast a vote; reviewer_b and reviewer_c have not executed/voted
    let mut context_state = HashMap::new();
    context_state.insert(
        "reviewer_a".to_string(),
        serde_json::json!({ "approved": true }),
    );

    let outcome = execute_consensus_gate_node(&gate, &context_state);

    assert_eq!(outcome.status, "failed");
    assert_eq!(outcome.node_output["consensus_reached"], false);
    assert_eq!(outcome.node_output["approved_votes"], 1);
    assert_eq!(outcome.node_output["missing_votes"], 2);
    assert_eq!(outcome.node_output["votes"]["reviewer_a"], "approved");
    assert_eq!(outcome.node_output["votes"]["reviewer_b"], "missing");
    assert_eq!(outcome.node_output["votes"]["reviewer_c"], "missing");
}

#[test]
fn test_consensus_gate_quorum_threshold_evaluation() {
    // 3 reviewers with 66% quorum -> ceil(3 * 0.66) = 2 required votes
    let gate_quorum = WorkflowNode::ConsensusGate(
        "gate_q",
        0, // will be overridden by quorum_threshold when required_votes is None or if configured
        vec![
            "rev_1".to_string(),
            "rev_2".to_string(),
            "rev_3".to_string(),
        ],
    );

    // Construct node where required_votes is None but quorum_threshold is Some(0.66)
    let mut node_with_quorum = gate_quorum;
    node_with_quorum.data.required_votes = None;
    node_with_quorum.data.quorum_threshold = Some(0.66);

    // Case 1: 2 approvals out of 3 -> meets 66% threshold
    let mut ctx_pass = HashMap::new();
    ctx_pass.insert("rev_1".to_string(), serde_json::json!(true));
    ctx_pass.insert("rev_2".to_string(), serde_json::json!("approved"));
    ctx_pass.insert("rev_3".to_string(), serde_json::json!(false));

    let outcome_pass = execute_consensus_gate_node(&node_with_quorum, &ctx_pass);
    assert_eq!(outcome_pass.status, "completed");
    assert_eq!(outcome_pass.node_output["consensus_reached"], true);
    assert_eq!(outcome_pass.node_output["required_votes"], 2);
    assert_eq!(outcome_pass.node_output["approved_votes"], 2);

    // Case 2: 1 approval out of 3 -> fails 66% threshold
    let mut ctx_fail = HashMap::new();
    ctx_fail.insert("rev_1".to_string(), serde_json::json!(true));
    ctx_fail.insert("rev_2".to_string(), serde_json::json!(false));
    ctx_fail.insert("rev_3".to_string(), serde_json::json!(false));

    let outcome_fail = execute_consensus_gate_node(&node_with_quorum, &ctx_fail);
    assert_eq!(outcome_fail.status, "failed");
    assert_eq!(outcome_fail.node_output["consensus_reached"], false);
    assert_eq!(outcome_fail.node_output["required_votes"], 2);
    assert_eq!(outcome_fail.node_output["approved_votes"], 1);
}

#[test]
fn test_consensus_gate_serde_roundtrip() {
    let node =
        WorkflowNode::ConsensusGate("gate_json", 2, vec!["alice".to_string(), "bob".to_string()])
            .with_label("Pair Approval")
            .with_quorum_threshold(0.75);

    let serialized = serde_json::to_string(&node).expect("Serialization must succeed");
    let deserialized: WorkflowNode =
        serde_json::from_str(&serialized).expect("Deserialization must succeed");

    assert_eq!(deserialized.id, "gate_json");
    assert_eq!(deserialized.node_type, "consensusGateNode");
    assert_eq!(deserialized.data.label.as_deref(), Some("Pair Approval"));
    assert_eq!(deserialized.data.required_votes, Some(2));
    assert_eq!(
        deserialized.data.reviewers,
        Some(vec!["alice".to_string(), "bob".to_string()])
    );
    assert_eq!(deserialized.data.quorum_threshold, Some(0.75));

    // Also test deserialization from raw camelCase JSON
    let raw_json = serde_json::json!({
        "id": "gate_camel",
        "type": "consensusGateNode",
        "data": {
            "requiredVotes": 3,
            "reviewers": ["reviewer_x", "reviewer_y", "reviewer_z"],
            "quorumThreshold": 0.66
        }
    });

    let from_json: WorkflowNode =
        serde_json::from_value(raw_json).expect("From Value must succeed");
    assert_eq!(from_json.data.required_votes, Some(3));
    assert_eq!(from_json.data.quorum_threshold, Some(0.66));
    assert_eq!(from_json.data.reviewers.unwrap().len(), 3);
}

#[tokio::test]
async fn test_orchestrator_consensus_gate_dag_sufficient_votes(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir =
        std::env::temp_dir().join(format!("tuzygent-consensus-pass-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    let dir_str = tmp_dir.to_str().ok_or("Failed to convert path")?;
    let orchestrator = WorkflowOrchestrator::new(dir_str, dir_str);

    // DAG:
    // root -> rev_1 \
    //                -> consensus_gate -> action_deploy
    // root -> rev_2 /
    let wf = test_workflow(
        serde_json::json!([
            { "id": "root", "type": "triggerNode", "data": { "triggerType": "manual" } },
            { "id": "rev_1", "type": "triggerNode", "data": {} },
            { "id": "rev_2", "type": "triggerNode", "data": {} },
            {
                "id": "consensus_gate",
                "type": "consensusGateNode",
                "data": {
                    "requiredVotes": 2,
                    "reviewers": ["rev_1", "rev_2"]
                }
            },
            { "id": "action_deploy", "type": "triggerNode", "data": {} }
        ]),
        serde_json::json!([
            { "id": "e1", "source": "root", "target": "rev_1" },
            { "id": "e2", "source": "root", "target": "rev_2" },
            { "id": "e3", "source": "rev_1", "target": "consensus_gate" },
            { "id": "e4", "source": "rev_2", "target": "consensus_gate" },
            { "id": "e5", "source": "consensus_gate", "target": "action_deploy" }
        ]),
    );

    // Provide votes for rev_1 and rev_2 via trigger payload
    let trigger_payload = serde_json::json!({
        "votes": {
            "rev_1": "approve",
            "rev_2": "approve"
        }
    });

    let report = orchestrator
        .execute_workflow(&wf, Some(trigger_payload), "test_runner")
        .await?;

    assert_eq!(report.status, "Completed");
    assert_eq!(report.node_results["consensus_gate"].status, "completed");
    assert_eq!(
        report.node_results["consensus_gate"].output["consensus_reached"],
        true
    );
    assert_eq!(
        report.node_results["consensus_gate"].output["approved_votes"],
        2
    );
    assert_eq!(report.node_results["action_deploy"].status, "completed");

    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}

#[tokio::test]
async fn test_orchestrator_consensus_gate_dag_insufficient_votes(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir =
        std::env::temp_dir().join(format!("tuzygent-consensus-fail-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    let dir_str = tmp_dir.to_str().ok_or("Failed to convert path")?;
    let orchestrator = WorkflowOrchestrator::new(dir_str, dir_str);

    // DAG:
    // root -> rev_1 \
    //                -> consensus_gate -> action_deploy
    // root -> rev_2 /
    let wf = test_workflow(
        serde_json::json!([
            { "id": "root", "type": "triggerNode", "data": { "triggerType": "manual" } },
            { "id": "rev_1", "type": "triggerNode", "data": {} },
            { "id": "rev_2", "type": "triggerNode", "data": {} },
            {
                "id": "consensus_gate",
                "type": "consensusGateNode",
                "data": {
                    "requiredVotes": 2,
                    "reviewers": ["rev_1", "rev_2"]
                }
            },
            { "id": "action_deploy", "type": "triggerNode", "data": {} }
        ]),
        serde_json::json!([
            { "id": "e1", "source": "root", "target": "rev_1" },
            { "id": "e2", "source": "root", "target": "rev_2" },
            { "id": "e3", "source": "rev_1", "target": "consensus_gate" },
            { "id": "e4", "source": "rev_2", "target": "consensus_gate" },
            { "id": "e5", "source": "consensus_gate", "target": "action_deploy" }
        ]),
    );

    // Provide only 1 approval; rev_2 rejects -> Insufficient votes!
    let trigger_payload = serde_json::json!({
        "votes": {
            "rev_1": "approve",
            "rev_2": "reject"
        }
    });

    let report = orchestrator
        .execute_workflow(&wf, Some(trigger_payload), "test_runner")
        .await?;

    assert_eq!(report.status, "Failed");
    assert_eq!(report.node_results["consensus_gate"].status, "failed");
    assert_eq!(
        report.node_results["consensus_gate"].output["consensus_reached"],
        false
    );
    assert_eq!(
        report.node_results["consensus_gate"].output["approved_votes"],
        1
    );
    assert_eq!(
        report.node_results["consensus_gate"].output["rejected_votes"],
        1
    );
    // Crucial: action_deploy must be skipped because consensus_gate failed!
    assert_eq!(report.node_results["action_deploy"].status, "skipped");

    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}
