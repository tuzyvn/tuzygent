use chrono::Utc;
use std::collections::HashMap;

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

fn parse_vote_value(val: &serde_json::Value) -> Option<bool> {
    match val {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::String(s) => {
            let norm = s.trim().to_lowercase();
            if norm == "approve"
                || norm == "approved"
                || norm == "yes"
                || norm == "pass"
                || norm == "true"
            {
                Some(true)
            } else if norm == "reject"
                || norm == "rejected"
                || norm == "no"
                || norm == "fail"
                || norm == "false"
            {
                Some(false)
            } else {
                None
            }
        }
        serde_json::Value::Object(map) => {
            for key in ["vote", "decision", "approved", "status", "consensus_vote"] {
                if let Some(v) = map.get(key) {
                    if let Some(res) = parse_vote_value(v) {
                        return Some(res);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

pub fn execute_consensus_gate_node(
    node: &WorkflowNode,
    context_state: &HashMap<String, serde_json::Value>,
) -> NodeExecutionOutcome {
    let reviewers = node.data.reviewers.clone().unwrap_or_default();
    let mut votes_map = serde_json::Map::new();
    let mut approved_count = 0usize;
    let mut rejected_count = 0usize;
    let mut missing_count = 0usize;

    // Check if there is an aggregated "votes" object in context_state or trigger
    let aggregated_votes = context_state
        .get("votes")
        .and_then(|v| v.as_object())
        .or_else(|| {
            context_state
                .get("trigger")
                .and_then(|t| t.get("votes"))
                .and_then(|v| v.as_object())
        });

    if !reviewers.is_empty() {
        for reviewer in &reviewers {
            let vote_val = aggregated_votes
                .and_then(|m| m.get(reviewer))
                .or_else(|| context_state.get(reviewer));

            let vote_status = match vote_val.and_then(parse_vote_value) {
                Some(true) => {
                    approved_count += 1;
                    "approved"
                }
                Some(false) => {
                    rejected_count += 1;
                    "rejected"
                }
                None => {
                    missing_count += 1;
                    "missing"
                }
            };
            votes_map.insert(reviewer.clone(), serde_json::json!(vote_status));
        }
    } else {
        // No explicit reviewers list given; collect votes from aggregated votes or context_state nodes
        if let Some(agg) = aggregated_votes {
            for (reviewer, val) in agg {
                match parse_vote_value(val) {
                    Some(true) => {
                        approved_count += 1;
                        votes_map.insert(reviewer.clone(), serde_json::json!("approved"));
                    }
                    Some(false) => {
                        rejected_count += 1;
                        votes_map.insert(reviewer.clone(), serde_json::json!("rejected"));
                    }
                    None => {
                        missing_count += 1;
                        votes_map.insert(reviewer.clone(), serde_json::json!("missing"));
                    }
                }
            }
        } else {
            for (key, val) in context_state {
                if key == "trigger" {
                    continue;
                }
                if let Some(parsed) = parse_vote_value(val) {
                    if parsed {
                        approved_count += 1;
                        votes_map.insert(key.clone(), serde_json::json!("approved"));
                    } else {
                        rejected_count += 1;
                        votes_map.insert(key.clone(), serde_json::json!("rejected"));
                    }
                }
            }
        }
    }

    let total_reviewers = if !reviewers.is_empty() {
        reviewers.len()
    } else {
        approved_count + rejected_count + missing_count
    };

    let required_votes = if let Some(req) = node.data.required_votes {
        req
    } else if let Some(threshold) = node.data.quorum_threshold {
        ((total_reviewers as f64) * threshold).ceil() as usize
    } else if total_reviewers > 0 {
        total_reviewers
    } else {
        1
    };

    let consensus_reached = approved_count >= required_votes;

    let node_output = serde_json::json!({
        "consensus_reached": consensus_reached,
        "decision": if consensus_reached { "Approved" } else { "Rejected" },
        "approved_votes": approved_count,
        "rejected_votes": rejected_count,
        "missing_votes": missing_count,
        "required_votes": required_votes,
        "total_reviewers": total_reviewers,
        "votes": votes_map,
    });

    if consensus_reached {
        NodeExecutionOutcome {
            node_output,
            node_error: None,
            engine_used: None,
            status: "completed".to_string(),
            healed: None,
            healing_attempts: None,
            lesson_learned: None,
        }
    } else {
        NodeExecutionOutcome {
            node_output,
            node_error: Some(format!(
                "Consensus not reached: required {} approved votes, received {} (total reviewers: {})",
                required_votes, approved_count, total_reviewers
            )),
            engine_used: None,
            status: "failed".to_string(),
            healed: None,
            healing_attempts: None,
            lesson_learned: None,
        }
    }
}
