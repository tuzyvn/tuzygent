use crate::bus::InterAgentBus;
use crate::sandbox::SandboxConfig;
use crate::supplement::InFlightSupplementController;
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
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
    pub(crate) checkpoint_store: Option<Arc<dyn crate::task_store::TaskStore>>,
    pub(crate) auto_approve: bool,
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
            checkpoint_store: None,
            auto_approve: false,
        }
    }

    pub fn with_auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    pub fn with_cancellation_token(mut self, token: Arc<AtomicBool>) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    pub fn with_checkpoint_store(mut self, store: Arc<dyn crate::task_store::TaskStore>) -> Self {
        self.checkpoint_store = Some(store);
        self
    }

    pub async fn checkpoint(&self, task: &agent_domain::Task) -> Result<(), String> {
        if let Some(store) = &self.checkpoint_store {
            store.save_checkpoint(&task.id, &task.subtasks).await.map_err(|e| format!("Failed to persist task checkpoint: {e}"))?;
        }
        Ok(())
    }

    pub async fn execute_engine(
        &self,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        crate::execution::ExecutionService::new(&self.workspace_dir, self.sandbox_config.clone(), Some(self.guardrail.clone()))
            .execute(self.engine.clone(), input).await
    }

    /// Khởi tạo ScopedScratchpad phân cấp cho subtask hoặc sub-agent được ủy quyền
    pub fn scoped_scratchpad(
        &self,
        sub_agent_or_subtask_id: &str,
        access: agent_memory::ScopeAccess,
    ) -> agent_memory::ScopedScratchpad {
        self.scratchpad.scoped_child(sub_agent_or_subtask_id, access)
    }

    /// Khởi tạo ScopedScratchpad cô lập hoàn toàn cho subtask hoặc sub-agent, cấm đọc trộm bộ nhớ cha
    pub fn isolated_scratchpad(
        &self,
        sub_agent_or_subtask_id: &str,
    ) -> agent_memory::ScopedScratchpad {
        self.scratchpad.isolated_child(sub_agent_or_subtask_id)
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
