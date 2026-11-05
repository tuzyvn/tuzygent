use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(default, alias = "required_votes", rename = "requiredVotes")]
    pub required_votes: Option<usize>,
    #[serde(default)]
    pub reviewers: Option<Vec<String>>,
    #[serde(default, alias = "quorum_threshold", rename = "quorumThreshold")]
    pub quorum_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub data: WorkflowNodeData,
}

impl WorkflowNode {
    #[allow(non_snake_case)]
    pub fn ConsensusGate(
        id: impl Into<String>,
        required_votes: usize,
        reviewers: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            node_type: "consensusGateNode".to_string(),
            data: WorkflowNodeData {
                required_votes: Some(required_votes),
                reviewers: Some(reviewers),
                ..Default::default()
            },
        }
    }

    pub fn consensus_gate(
        id: impl Into<String>,
        required_votes: usize,
        reviewers: Vec<String>,
    ) -> Self {
        Self::ConsensusGate(id, required_votes, reviewers)
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.data.label = Some(label.into());
        self
    }

    pub fn with_quorum_threshold(mut self, threshold: f64) -> Self {
        self.data.quorum_threshold = Some(threshold);
        self
    }
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

pub type WorkflowDag = WorkflowDefinition;

impl WorkflowDefinition {
    /// Render workflow DAG as a Mermaid diagram (`graph TD`).
    pub fn to_mermaid(&self) -> String {
        let mut out = String::from("graph TD\n");
        for node in &self.nodes {
            if let Some(label) = &node.data.label {
                if !label.is_empty() {
                    out.push_str(&format!("    {}[\"{}\"]\n", node.id, label));
                } else {
                    out.push_str(&format!("    {}\n", node.id));
                }
            } else {
                out.push_str(&format!("    {}\n", node.id));
            }
        }
        for edge in &self.edges {
            if let Some(label) = &edge.label {
                if !label.is_empty() {
                    out.push_str(&format!(
                        "    {} -->|{}| {}\n",
                        edge.source, label, edge.target
                    ));
                } else {
                    out.push_str(&format!("    {} --> {}\n", edge.source, edge.target));
                }
            } else {
                out.push_str(&format!("    {} --> {}\n", edge.source, edge.target));
            }
        }
        out
    }

    /// Render workflow DAG as a Graphviz DOT diagram (`digraph { ... }`).
    pub fn to_dot(&self) -> String {
        let mut out = String::from("digraph {\n");
        for node in &self.nodes {
            if let Some(label) = &node.data.label {
                if !label.is_empty() {
                    out.push_str(&format!("    {} [label=\"{}\"];\n", node.id, label));
                } else {
                    out.push_str(&format!("    {};\n", node.id));
                }
            } else {
                out.push_str(&format!("    {};\n", node.id));
            }
        }
        for edge in &self.edges {
            if let Some(label) = &edge.label {
                if !label.is_empty() {
                    out.push_str(&format!(
                        "    {} -> {} [label=\"{}\"];\n",
                        edge.source, edge.target, label
                    ));
                } else {
                    out.push_str(&format!("    {} -> {};\n", edge.source, edge.target));
                }
            } else {
                out.push_str(&format!("    {} -> {};\n", edge.source, edge.target));
            }
        }
        out.push('}');
        out
    }
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
