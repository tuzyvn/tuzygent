use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum number of teammates allowed in a single `coordinate_bots` handoff (OpenMausBot guard).
pub const MAX_COORDINATE_BOTS: usize = 4;

/// Default maximum delegation nesting depth before requiring agents to report back up the chain.
pub const DEFAULT_MAX_COORDINATION_DEPTH: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeatStatusReport {
    pub seat_id: String,
    pub name: String,
    pub role: String,
    pub engine: String,
    pub harness: String,
    pub is_active: bool,
    pub session_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigSeatAssignment {
    pub seat_id: String,
    pub task: String,
    pub status: String,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigExecutionSummary {
    pub rig_id: String,
    pub rig_name: String,
    pub operator_id: String,
    pub goal: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: String,
    pub assignments: Vec<RigSeatAssignment>,
    #[serde(default)]
    pub channel_dispatches: Vec<RigSeatAssignment>,
    pub operator_synthesis: String,
}

/// Public seat metadata returned by `list_room_targets` without leaking private transcripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTargetInfo {
    pub seat_id: String,
    pub name: String,
    pub role: String,
    pub engine: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Reply policy for `coordinate_bots`, matching OpenMausBot's wait-and-resume vs one-way notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationReplyMode {
    #[default]
    WaitAndResume,
    None,
}

fn default_max_depth() -> u32 {
    DEFAULT_MAX_COORDINATION_DEPTH
}

/// Delegation context tracking thread identity, nesting depth, and ancestor anti-cycle chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinationContext {
    pub thread_id: String,
    #[serde(default)]
    pub depth: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default)]
    pub ancestors: Vec<String>,
}

impl Default for CoordinationContext {
    fn default() -> Self {
        Self {
            thread_id: "default-thread".to_string(),
            depth: 0,
            max_depth: DEFAULT_MAX_COORDINATION_DEPTH,
            ancestors: Vec::new(),
        }
    }
}

impl CoordinationContext {
    /// Create a fresh root coordination context for the given thread identifier.
    pub fn new(thread_id: &str) -> Self {
        Self {
            thread_id: thread_id.trim().to_string(),
            ..Self::default()
        }
    }

    /// Override the maximum allowed handoff depth.
    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Seed the ancestor chain with upstream callers that must never be targeted recursively.
    pub fn with_ancestors(mut self, ancestors: Vec<String>) -> Self {
        self.ancestors = ancestors;
        self
    }

    /// Derive the child context for a handoff from `caller_id`, enforcing `depth < max_depth`.
    pub fn child_for(&self, caller_id: &str) -> Result<Self, String> {
        if self.depth >= self.max_depth {
            return Err(format!(
                "Maximum coordination depth ({}) reached; finish your turn and report back to your caller.",
                self.max_depth
            ));
        }
        let caller = caller_id.trim().trim_start_matches('@');
        let mut next_ancestors = self.ancestors.clone();
        if !caller.is_empty()
            && !next_ancestors
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(caller))
        {
            next_ancestors.push(caller.to_string());
        }
        Ok(Self {
            thread_id: self.thread_id.clone(),
            depth: self.depth + 1,
            max_depth: self.max_depth,
            ancestors: next_ancestors,
        })
    }
}

/// Request payload for `KernelOperator::coordinate_bots` and `coordinate_and_resume`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateBotsRequest {
    pub caller_id: String,
    pub bot_ids: Vec<String>,
    pub instructions: String,
    #[serde(default)]
    pub done_when: Option<String>,
    #[serde(default)]
    pub consult: bool,
    #[serde(default)]
    pub rework: bool,
    #[serde(default)]
    pub reply_mode: CoordinationReplyMode,
}

/// Per-teammate execution outcome produced by `coordinate_bots`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeammateTurnResult {
    pub seat_id: String,
    pub name: String,
    pub role: String,
    pub status: String,
    pub output: String,
    pub done_when_satisfied: bool,
    pub child_depth: u32,
    #[serde(default)]
    pub env_metadata: HashMap<String, String>,
}

/// Aggregated result of a `coordinate_bots` or `coordinate_and_resume` batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinationBatchResult {
    pub rig_id: String,
    pub thread_id: String,
    pub caller_id: String,
    pub depth: u32,
    pub consult: bool,
    pub rework: bool,
    pub reply_mode: CoordinationReplyMode,
    pub results: Vec<TeammateTurnResult>,
    #[serde(default)]
    pub resume_prompt_for_caller: Option<String>,
    #[serde(default)]
    pub caller_resume_output: Option<String>,
}
