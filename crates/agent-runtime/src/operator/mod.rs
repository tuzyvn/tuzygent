mod execution;
mod seat_lifecycle;
mod snapshot;
mod types;

#[cfg(test)]
mod tests;

pub use types::{RigExecutionSummary, RigSeatAssignment, SeatStatusReport};

use agent_domain::RigSpec;
use agent_engine::ComputeEngine;
use std::sync::Arc;

pub struct KernelOperator {
    pub spec: RigSpec,
    pub workspace_dir: String,
    pub(crate) engines: std::collections::HashMap<String, Arc<dyn ComputeEngine>>,
}

impl KernelOperator {
    pub fn new(spec: RigSpec, workspace_dir: &str) -> Self {
        Self {
            spec,
            workspace_dir: workspace_dir.to_string(),
            engines: Default::default(),
        }
    }

    /// Register an SDK-provided compute engine for this operator.
    pub fn with_engine(mut self, name: &str, engine: Arc<dyn ComputeEngine>) -> Self {
        self.engines.insert(name.to_lowercase(), engine);
        self
    }
}
