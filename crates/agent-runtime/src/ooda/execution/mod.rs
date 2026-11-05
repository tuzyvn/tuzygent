use crate::bus::InterAgentBus;
use crate::sandbox::{SandboxConfig, SandboxEnvironment};
use crate::supplement::InFlightSupplementController;
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use std::path::Path;
use std::sync::Arc;

pub const MAX_SUBTASK_RETRIES: u32 = 3;
pub const MAX_TOTAL_REMEDIATIONS: usize = 6;

pub mod git;
pub mod remediation;
pub mod runner;
pub mod safety;
pub mod verification;

#[cfg(test)]
mod runner_tests;

pub use git::capture_git_diff;
pub use remediation::{
    attempt_remediation_retry, generate_remediation_subtask, parse_remediation_json,
    RemediationFailureKind, RemediationOutcome,
};
pub use runner::{
    build_engine_input, execute_queue, execute_single_step, publish_subtask_executing,
    update_working_context, QueueExecutionOutcome, StepAction,
};
pub use safety::{
    check_and_apply_guardrail_policy, evaluate_command_safety, evaluate_guardrail_safety,
    GuardrailBlockReason,
};
pub use verification::{
    execute_subtask_verification, finalize_subtask_completion, SubtaskVerificationOutcome,
};
use std::sync::atomic::AtomicBool;

/// Executor responsible for running subtasks, verifying outcomes,
/// handling failures, and generating dynamic remediations.
#[derive(Clone)]
pub struct SubtaskExecutor {
    pub(crate) engine: Arc<dyn ComputeEngine>,
    pub(crate) guardrail: GuardrailPolicy,
    pub(crate) scratchpad: ScratchpadManager,
    pub(crate) durable: DurableMemoryStore,
    pub(crate) workspace_dir: String,
    pub(crate) supplement_ctrl: InFlightSupplementController,
    pub(crate) sandbox_config: Option<SandboxConfig>,
    pub(crate) bus: Option<InterAgentBus>,
    pub(crate) timeout_seconds: u64,
    pub(crate) cancellation_token: Option<Arc<AtomicBool>>,
}

impl SubtaskExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: Arc<dyn ComputeEngine>,
        guardrail: GuardrailPolicy,
        scratchpad: ScratchpadManager,
        durable: DurableMemoryStore,
        workspace_dir: impl Into<String>,
        supplement_ctrl: InFlightSupplementController,
        sandbox_config: Option<SandboxConfig>,
        bus: Option<InterAgentBus>,
        timeout_seconds: u64,
    ) -> Self {
        Self {
            engine,
            guardrail,
            scratchpad,
            durable,
            workspace_dir: workspace_dir.into(),
            supplement_ctrl,
            sandbox_config,
            bus,
            timeout_seconds,
            cancellation_token: None,
        }
    }

    pub fn with_cancellation_token(mut self, token: Arc<AtomicBool>) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    pub async fn execute_engine(
        &self,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let mut input = input.clone();
        let config = self.sandbox_config.clone().unwrap_or_default();
        input.command_sandbox = SandboxEnvironment::with_config(&self.workspace_dir, config)
            .command_sandbox(Path::new(&input.working_dir));
        self.engine.execute(&input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(MAX_SUBTASK_RETRIES, 3);
        assert_eq!(MAX_TOTAL_REMEDIATIONS, 6);
    }
}
