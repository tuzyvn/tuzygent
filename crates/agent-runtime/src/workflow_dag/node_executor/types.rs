use std::collections::HashMap;

use crate::workflow_dag::types::WorkflowTimelineEvent;

#[derive(Debug, Clone)]
pub struct NodeExecutionOutcome {
    pub node_output: serde_json::Value,
    pub node_error: Option<String>,
    pub engine_used: Option<String>,
    pub status: String,
    pub healed: Option<bool>,
    pub healing_attempts: Option<u32>,
    pub lesson_learned: Option<String>,
}

pub struct NodeExecutionContext<'a> {
    pub context_state: &'a HashMap<String, serde_json::Value>,
    pub last_agent_execution: &'a mut Option<(String, String)>,
    pub depth: usize,
    pub timeline: &'a mut Vec<WorkflowTimelineEvent>,
}
