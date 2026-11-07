use crate::harness::HarnessMemoryStore;
use agent_engine::ComputeEngine;
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::node_executor::{execute_node, NodeExecutionContext, NodeExecutionOutcome};
use super::types::{
    WorkflowDefinition, WorkflowNode, WorkflowNodeExecutionResult, WorkflowRunReport,
    WorkflowTimelineEvent,
};
use super::validation::validate_workflow;

pub struct WorkflowOrchestrator {
    pub(crate) workspace_dir: String,
    pub(crate) _personas_dir: String,
    pub(crate) harness_memory: HarnessMemoryStore,
}

impl WorkflowOrchestrator {
    pub fn new(workspace_dir: &str, personas_dir: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_string(),
            _personas_dir: personas_dir.to_string(),
            harness_memory: HarnessMemoryStore::new(workspace_dir),
        }
    }

    pub(crate) fn resolve_engine(
        &self,
        engine_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<Arc<dyn ComputeEngine>> {
        let config = agent_engine::EnginesConfig::load_hierarchical(Some(std::path::Path::new(
            &self.workspace_dir,
        )));
        Ok(agent_engine::configured_engine(
            &config,
            engine_name.unwrap_or(&config.defaults.engine),
            model,
        )?)
    }

    pub async fn load_workflow_from_catalog(
        &self,
        workflow_id: &str,
    ) -> Result<WorkflowDefinition> {
        let catalog_path = std::path::Path::new(&self.workspace_dir)
            .join("workflows")
            .join("catalog.json");
        let data = tokio::fs::read(&catalog_path)
            .await
            .map_err(|e| anyhow!("Không đọc được file catalog tại {:?}: {}", catalog_path, e))?;
        let workflows: Vec<WorkflowDefinition> =
            serde_json::from_slice(&data).map_err(|e| anyhow!("Lỗi parse JSON catalog: {}", e))?;
        workflows
            .into_iter()
            .find(|w| w.id == workflow_id)
            .ok_or_else(|| {
                anyhow!(
                    "Không tìm thấy Sub-Workflow có ID '{}' trong catalog",
                    workflow_id
                )
            })
    }

    pub async fn execute_workflow(
        &self,
        workflow: &WorkflowDefinition,
        trigger_payload: Option<serde_json::Value>,
        triggered_by: &str,
    ) -> Result<WorkflowRunReport> {
        self.execute_workflow_internal(workflow, trigger_payload, triggered_by, 0)
            .await
    }

    pub fn execute_workflow_internal<'a>(
        &'a self,
        workflow: &'a WorkflowDefinition,
        trigger_payload: Option<serde_json::Value>,
        triggered_by: &'a str,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<WorkflowRunReport>> + Send + 'a>>
    {
        Box::pin(async move {
            if depth > 5 {
                return Err(anyhow!(
                    "Vượt quá độ sâu tối đa (5) của Sub-Workflow lồng nhau"
                ));
            }

            validate_workflow(workflow)?;
            let run_id = format!("run-{}", uuid::Uuid::new_v4().simple());
            let start_time = Utc::now();
            let mut timeline: Vec<WorkflowTimelineEvent> = Vec::new();
            let mut node_results: HashMap<String, WorkflowNodeExecutionResult> = HashMap::new();
            let mut context_state: HashMap<String, serde_json::Value> = HashMap::new();

            if let Some(payload) = trigger_payload {
                context_state.insert("trigger".to_string(), payload);
            }

            timeline.push(WorkflowTimelineEvent {
                timestamp: Utc::now().format("%H:%M:%S").to_string(),
                level: "info".to_string(),
                message: format!(
                    "Khởi chạy Workflow '{}' (ID: {}, Depth: {})",
                    workflow.name, workflow.id, depth
                ),
            });

            // Xây dựng DAG (In-degree & Adjacency)
            let mut in_degree: HashMap<String, usize> = HashMap::new();
            let mut adj: HashMap<String, Vec<String>> = HashMap::new();
            let mut node_map: HashMap<String, &WorkflowNode> = HashMap::new();

            for node in &workflow.nodes {
                in_degree.insert(node.id.clone(), 0);
                adj.insert(node.id.clone(), Vec::new());
                node_map.insert(node.id.clone(), node);
            }

            for edge in &workflow.edges {
                if let Some(deg) = in_degree.get_mut(&edge.target) {
                    *deg += 1;
                } else {
                    tracing::warn!(target = %edge.target, "Workflow edge references unknown target node");
                }
                if let Some(neighbors) = adj.get_mut(&edge.source) {
                    neighbors.push(edge.target.clone());
                } else {
                    tracing::warn!(source = %edge.source, "Workflow edge references unknown source node");
                }
            }

            // Hàng đợi Topological sort
            let mut queue: VecDeque<String> = VecDeque::new();
            for (id, deg) in &in_degree {
                if *deg == 0 {
                    queue.push_back(id.clone());
                }
            }

            // Theo dõi phiên chạy của Agent trước đó để chuyển giao ngữ cảnh chéo engine (Cross-Engine Handoff)
            let mut last_agent_execution: Option<(String, String)> = None;

            while let Some(current_id) = queue.pop_front() {
                let node = match node_map.get(&current_id) {
                    Some(n) => *n,
                    None => continue,
                };

                let node_label = node
                    .data
                    .label
                    .clone()
                    .unwrap_or_else(|| current_id.clone());
                let node_start = Utc::now();

                timeline.push(WorkflowTimelineEvent {
                    timestamp: node_start.format("%H:%M:%S").to_string(),
                    level: "info".to_string(),
                    message: format!("Thực thi Node '{}' ({})", node_label, node.node_type),
                });

                let dependencies_complete = workflow
                    .edges
                    .iter()
                    .filter(|edge| edge.target == current_id)
                    .all(|edge| {
                        node_results
                            .get(&edge.source)
                            .is_some_and(|result| result.status == "completed")
                    });

                let outcome = if !dependencies_complete {
                    NodeExecutionOutcome {
                        node_output: serde_json::json!({}),
                        node_error: Some("A prerequisite failed or is waiting for approval".into()),
                        engine_used: None,
                        status: "skipped".into(),
                        healed: None,
                        healing_attempts: None,
                        lesson_learned: None,
                    }
                } else {
                    let mut node_ctx = NodeExecutionContext {
                        context_state: &context_state,
                        last_agent_execution: &mut last_agent_execution,
                        depth,
                        timeline: &mut timeline,
                    };
                    match execute_node(self, workflow, node, &current_id, &mut node_ctx).await {
                        Ok(o) => o,
                        Err(err) => {
                            let err_msg = format!("Lỗi thực thi Node: {}", err);
                            timeline.push(WorkflowTimelineEvent {
                                timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                level: "error".to_string(),
                                message: err_msg.clone(),
                            });
                            NodeExecutionOutcome {
                                node_output: serde_json::json!({}),
                                node_error: Some(err_msg),
                                engine_used: None,
                                status: "failed".into(),
                                healed: None,
                                healing_attempts: None,
                                lesson_learned: None,
                            }
                        }
                    }
                };

                let node_finish = Utc::now();
                let duration_ms = (node_finish - node_start).num_milliseconds().max(0) as u128;

                // Lưu kết quả node vào context chung
                context_state.insert(current_id.clone(), outcome.node_output.clone());

                node_results.insert(
                    current_id.clone(),
                    WorkflowNodeExecutionResult {
                        node_id: current_id.clone(),
                        node_name: node_label.clone(),
                        node_type: node.node_type.clone(),
                        status: outcome.status,
                        started_at: node_start.to_rfc3339(),
                        finished_at: node_finish.to_rfc3339(),
                        execution_ms: duration_ms,
                        engine_used: outcome.engine_used,
                        input: serde_json::json!({ "context_keys": context_state.keys().collect::<Vec<_>>() }),
                        output: outcome.node_output,
                        error: outcome.node_error,
                        healed: outcome.healed,
                        healing_attempts: outcome.healing_attempts,
                        lesson_learned: outcome.lesson_learned,
                    },
                );

                // Mở khóa các node tiếp theo
                if let Some(neighbors) = adj.get(&current_id) {
                    for neighbor in neighbors {
                        if let Some(deg) = in_degree.get_mut(neighbor) {
                            *deg = deg.saturating_sub(1);
                            if *deg == 0 {
                                queue.push_back(neighbor.clone());
                            }
                        }
                    }
                }
            }

            let finish_time = Utc::now();
            let total_duration = (finish_time - start_time).num_milliseconds().max(0) as u128;

            timeline.push(WorkflowTimelineEvent {
                timestamp: finish_time.format("%H:%M:%S").to_string(),
                level: "info".to_string(),
                message: format!(
                    "Workflow '{}' hoàn tất trong {}ms",
                    workflow.name, total_duration
                ),
            });

            Ok(WorkflowRunReport {
                run_id,
                workflow_id: workflow.id.clone(),
                workflow_name: workflow.name.clone(),
                workflow_version: workflow
                    .version
                    .clone()
                    .unwrap_or_else(|| "1.0.0".to_string()),
                triggered_by: triggered_by.to_string(),
                trigger_type: "DAG_Orchestrator".to_string(),
                status: if node_results.values().any(|node| node.status == "failed") {
                    "Failed"
                } else if node_results
                    .values()
                    .any(|node| node.status == "waiting_approval")
                {
                    "WaitingApproval"
                } else if node_results.len() == workflow.nodes.len()
                    && node_results.values().all(|node| node.status == "completed")
                {
                    "Completed"
                } else {
                    "Failed"
                }
                .into(),
                started_at: start_time.to_rfc3339(),
                completed_at: finish_time.to_rfc3339(),
                duration_ms: total_duration,
                total_tokens_used: 0,
                node_results,
                timeline,
            })
        })
    }
}
