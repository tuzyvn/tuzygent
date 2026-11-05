use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeData {
    pub label: Option<String>,
    #[serde(rename = "personaId")]
    pub persona_id: Option<String>,
    #[serde(rename = "personaName")]
    pub persona_name: Option<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub role: Option<String>,
    #[serde(rename = "promptTemplate")]
    pub prompt_template: Option<String>,
    #[serde(rename = "triggerType")]
    pub trigger_type: Option<String>,
    pub endpoint: Option<String>,
    #[serde(rename = "cronExpr")]
    pub cron_expr: Option<String>,
    pub expression: Option<String>,
    #[serde(rename = "requiredRole")]
    pub required_role: Option<String>,
    pub command: Option<String>,
    #[serde(rename = "subWorkflowId")]
    pub sub_workflow_id: Option<String>,
    #[serde(rename = "subWorkflowName")]
    pub sub_workflow_name: Option<String>,
    #[serde(rename = "enableSelfHealing")]
    pub enable_self_healing: Option<bool>,
    #[serde(rename = "maxHealingAttempts")]
    pub max_healing_attempts: Option<u32>,
    pub healed: Option<bool>,
    #[serde(rename = "healingAttempts")]
    pub healing_attempts: Option<u32>,
    #[serde(rename = "lessonLearned")]
    pub lesson_learned: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub data: WorkflowNodeData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "sourceHandle")]
    pub source_handle: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeExecutionResult {
    pub node_id: String,
    pub node_name: String,
    pub node_type: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
    pub execution_ms: u128,
    pub engine_used: Option<String>,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub healed: Option<bool>,
    pub healing_attempts: Option<u32>,
    pub lesson_learned: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunReport {
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub workflow_version: String,
    pub triggered_by: String,
    pub trigger_type: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u128,
    pub total_tokens_used: u64,
    pub node_results: HashMap<String, WorkflowNodeExecutionResult>,
    pub timeline: Vec<WorkflowTimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTimelineEvent {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}
