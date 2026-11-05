use serde::{Deserialize, Serialize};

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
