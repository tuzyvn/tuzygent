mod coordinate;
mod coordinate_prompts;
mod execution;
mod seat_lifecycle;
mod snapshot;
mod types;

#[cfg(test)]
mod tests;

pub use types::{
    CoordinateBotsRequest, CoordinationBatchResult, CoordinationContext, CoordinationReplyMode,
    RigExecutionSummary, RigSeatAssignment, RoomTargetInfo, SeatStatusReport, TeammateTurnResult,
    DEFAULT_MAX_COORDINATION_DEPTH, MAX_COORDINATE_BOTS,
};

use agent_domain::{
    discover_workspace_roster, load_subagents_from_dir, roster_to_rig_spec, RigSpec, SubagentSpec,
};
use agent_engine::ComputeEngine;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub struct KernelOperator {
    pub spec: RigSpec,
    pub workspace_dir: String,
    pub(crate) engines: HashMap<String, Arc<dyn ComputeEngine>>,
    pub(crate) subagents: HashMap<String, SubagentSpec>,
}

impl KernelOperator {
    pub fn new(spec: RigSpec, workspace_dir: &str) -> Self {
        Self {
            spec,
            workspace_dir: workspace_dir.to_string(),
            engines: Default::default(),
            subagents: Default::default(),
        }
    }

    /// Build a modern `KernelOperator` directly from parsed Markdown subagent rosters (`agents/*.md`).
    pub fn from_subagents(
        rig_id: &str,
        rig_name: &str,
        specs: Vec<SubagentSpec>,
        workspace_dir: &str,
    ) -> Self {
        let rig = roster_to_rig_spec(rig_id, rig_name, &specs);
        let subagents = specs
            .into_iter()
            .map(|s| (s.id.to_ascii_lowercase(), s))
            .collect();
        Self {
            spec: rig,
            workspace_dir: workspace_dir.to_string(),
            engines: Default::default(),
            subagents,
        }
    }

    /// Zero-config discovery from `agents/`, `.tuzygent/agents/`, or `.claude/agents/`, falling back to built-in defaults.
    pub fn from_workspace(workspace_dir: &str) -> Result<Self, String> {
        let (specs, rig) = discover_workspace_roster(Path::new(workspace_dir))?;
        let subagents = specs
            .into_iter()
            .map(|s| (s.id.to_ascii_lowercase(), s))
            .collect();
        Ok(Self {
            spec: rig,
            workspace_dir: workspace_dir.to_string(),
            engines: Default::default(),
            subagents,
        })
    }

    /// Load roster from an `agents/` directory, a `.md` subagent file, a legacy YAML `RigSpec`, or fall back to workspace discovery.
    pub fn load_from_path_or_workspace(spec_path: &Path, workspace_dir: &str) -> Result<Self, String> {
        let resolved = if spec_path.exists() {
            spec_path.to_path_buf()
        } else {
            let joined = Path::new(workspace_dir).join(spec_path);
            if joined.exists() {
                joined
            } else {
                spec_path.to_path_buf()
            }
        };
        if resolved.is_dir() {
            let specs = load_subagents_from_dir(&resolved)?;
            if specs.is_empty() {
                return Self::from_workspace(workspace_dir);
            }
            let dir_name = resolved
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("workspace-roster");
            return Ok(Self::from_subagents(dir_name, "Markdown Subagent Roster", specs, workspace_dir));
        }
        if resolved.is_file() {
            let content = std::fs::read_to_string(&resolved)
                .map_err(|e| format!("Failed to read spec '{}': {e}", resolved.display()))?;
            if resolved.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")) {
                let fallback = resolved
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("subagent");
                let source = resolved.to_string_lossy().to_string();
                let spec = SubagentSpec::from_markdown_str(fallback, &content, Some(&source))?;
                let (rig_id, rig_name) = (spec.id.clone(), spec.name.clone());
                return Ok(Self::from_subagents(&rig_id, &rig_name, vec![spec], workspace_dir));
            }
            let rig_spec = RigSpec::from_yaml_str(&content)
                .map_err(|e| format!("Invalid YAML RigSpec '{}': {e}", resolved.display()))?;
            return Ok(Self::new(rig_spec, workspace_dir));
        }
        Self::from_workspace(workspace_dir)
    }

    /// Resolve isolated Markdown system prompt for `seat_id` if configured.
    pub fn seat_system_prompt(&self, seat_id: &str) -> Option<String> {
        if let Some(sp) = self.subagents.get(&seat_id.to_ascii_lowercase()) {
            if !sp.system_prompt.trim().is_empty() {
                return Some(sp.system_prompt.clone());
            }
        }
        let path_str = self
            .spec
            .find_seat(seat_id)
            .and_then(|seat| seat.persona_path.as_deref())?;
        let path = Path::new(path_str);
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")) {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(parsed) = SubagentSpec::from_markdown_str(seat_id, &content, Some(path_str)) {
                    if !parsed.system_prompt.trim().is_empty() {
                        return Some(parsed.system_prompt);
                    }
                }
            }
        }
        None
    }

    /// Register an SDK-provided compute engine for this operator.
    pub fn with_engine(mut self, name: &str, engine: Arc<dyn ComputeEngine>) -> Self {
        self.engines.insert(name.to_lowercase(), engine);
        self
    }
}
