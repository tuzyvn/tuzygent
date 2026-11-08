mod context;
mod decomposition;
pub mod execution;
mod lifecycle;

pub use decomposition::GoalDecomposer;
pub use execution::{
    QueueExecutionOutcome, SubtaskExecutor, MAX_SUBTASK_RETRIES, MAX_TOTAL_REMEDIATIONS,
};

pub const OODA_PHASE_OBSERVE: &str = "Observe";
pub const OODA_PHASE_ORIENT: &str = "Orient";
pub const OODA_PHASE_DECIDE: &str = "Decide";
pub const OODA_PHASE_ACT: &str = "Act";
pub const OODA_PHASE_REMEDIATE: &str = "Remediate";

use crate::bus::InterAgentBus;
use crate::sandbox::SandboxConfig;
use crate::supplement::InFlightSupplementController;
use agent_domain::GuardrailPolicy;
use agent_engine::ComputeEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use std::sync::Arc;

pub(crate) async fn wait_for_cancellation(token: &Option<Arc<std::sync::atomic::AtomicBool>>) {
    if let Some(token) = token {
        while !token.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    } else {
        std::future::pending::<()>().await;
    }
}

/// Autonomous OODA Loop Facade
///
/// Coordinates sandbox preparation, agent briefing, memory retrieval,
/// goal decomposition (`GoalDecomposer`), subtask execution & dynamic
/// backtracking (`SubtaskExecutor`), and episode recording.
pub struct AutonomousOodaLoop {
    pub(crate) engine: Arc<dyn ComputeEngine>,
    pub(crate) guardrail: GuardrailPolicy,
    pub(crate) scratchpad: ScratchpadManager,
    pub(crate) durable: DurableMemoryStore,
    pub(crate) workspace_dir: String,
    pub(crate) supplement_ctrl: InFlightSupplementController,
    pub(crate) sandbox_config: Option<SandboxConfig>,
    pub(crate) bus: Option<InterAgentBus>,
    pub(crate) timeout_seconds: u64,
    pub(crate) persona: Option<agent_domain::AgentPersonaProfile>,
    pub(crate) cancellation_token: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) checkpoint_store: Option<Arc<dyn crate::task_store::TaskStore>>,
    pub(crate) auto_approve: bool,
    pub(crate) backoff_policy: Option<agent_engine::BackoffPolicy>,
}

impl AutonomousOodaLoop {
    pub fn new(
        engine: Arc<dyn ComputeEngine>,
        guardrail: GuardrailPolicy,
        scratchpad: ScratchpadManager,
        durable: DurableMemoryStore,
        workspace_dir: &str,
    ) -> Self {
        Self {
            engine,
            guardrail,
            scratchpad,
            durable,
            workspace_dir: workspace_dir.to_string(),
            supplement_ctrl: InFlightSupplementController::new(16),
            sandbox_config: None,
            bus: None,
            timeout_seconds: 300,
            persona: None,
            cancellation_token: None,
            checkpoint_store: None,
            auto_approve: false,
            backoff_policy: None,
        }
    }

    pub fn with_backoff_policy(mut self, policy: agent_engine::BackoffPolicy) -> Self {
        self.backoff_policy = Some(policy);
        self
    }

    pub fn backoff_policy(&self) -> Option<&agent_engine::BackoffPolicy> {
        self.backoff_policy.as_ref()
    }

    pub fn with_auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    pub fn with_persona(mut self, profile: agent_domain::AgentPersonaProfile) -> Self {
        self.persona = Some(profile);
        self
    }

    pub fn with_cancellation_token(mut self, token: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    pub fn with_checkpoint_store(mut self, store: Arc<dyn crate::task_store::TaskStore>) -> Self {
        self.checkpoint_store = Some(store);
        self
    }

    pub fn with_timeout_seconds(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds.max(1);
        self
    }

    pub fn with_bus(mut self, bus: InterAgentBus) -> Self {
        self.bus = Some(bus);
        self
    }

    pub fn with_sandbox_config(mut self, config: SandboxConfig) -> Self {
        self.sandbox_config = Some(config);
        self
    }

    pub fn supplement_controller(&self) -> InFlightSupplementController {
        self.supplement_ctrl.clone()
    }

    pub fn decomposer(&self) -> GoalDecomposer {
        let mut decomposer = GoalDecomposer::new(
            self.engine.clone(),
            self.guardrail.clone(),
            &self.workspace_dir,
            self.sandbox_config.clone(),
            self.timeout_seconds,
        );
        if let Some(ref policy) = self.backoff_policy {
            decomposer = decomposer.with_backoff_policy(policy.clone());
        }
        decomposer
    }

    pub fn executor(&self) -> SubtaskExecutor {
        let mut exec = SubtaskExecutor::new(
            self.engine.clone(),
            self.guardrail.clone(),
            self.scratchpad.clone(),
            self.durable.clone(),
            &self.workspace_dir,
            self.supplement_ctrl.clone(),
            self.sandbox_config.clone(),
            self.bus.clone(),
            self.timeout_seconds,
        );
        if let Some(ref token) = self.cancellation_token {
            exec = exec.with_cancellation_token(token.clone());
        }
        if let Some(ref store) = self.checkpoint_store {
            exec = exec.with_checkpoint_store(store.clone());
        }
        if self.auto_approve {
            exec = exec.with_auto_approve(true);
        }
        if let Some(ref policy) = self.backoff_policy {
            exec = exec.with_backoff_policy(policy.clone());
        }
        exec
    }

    /// Khởi tạo ScopedScratchpad phân cấp cho sub-agent được ủy quyền
    pub fn scoped_scratchpad(
        &self,
        sub_agent_or_subtask_id: &str,
        access: agent_memory::ScopeAccess,
    ) -> agent_memory::ScopedScratchpad {
        self.scratchpad
            .scoped_child(sub_agent_or_subtask_id, access)
    }

    /// Khởi tạo ScopedScratchpad cô lập hoàn toàn cho sub-agent, cấm đọc trộm bộ nhớ cha
    pub fn isolated_scratchpad(
        &self,
        sub_agent_or_subtask_id: &str,
    ) -> agent_memory::ScopedScratchpad {
        self.scratchpad.isolated_child(sub_agent_or_subtask_id)
    }
}

#[cfg(test)]
mod public_api_tests {
    use super::AutonomousOodaLoop;
    use agent_domain::{
        AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul,
        AutonomyLevel, GuardrailPolicy,
    };
    use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
    use agent_memory::{DurableMemoryStore, ScratchpadManager};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct UnusedEngine;

    #[async_trait]
    impl ComputeEngine for UnusedEngine {
        async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
            Err(EngineError::ProcessFailed(
                "engine execution is not expected in public API tests".into(),
            ))
        }

        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }

    fn test_loop() -> AutonomousOodaLoop {
        let workspace = std::env::temp_dir().join(format!(
            "agent_runtime_ooda_public_api_{}",
            uuid::Uuid::new_v4()
        ));
        let workspace_dir = workspace.to_string_lossy();
        AutonomousOodaLoop::new(
            Arc::new(UnusedEngine),
            GuardrailPolicy::default_enterprise(),
            ScratchpadManager::new(&workspace),
            DurableMemoryStore::new(&workspace),
            &workspace_dir,
        )
    }

    #[tokio::test]
    async fn unused_engine_returns_an_error_instead_of_panicking() {
        let ooda = test_loop();

        let result = ooda.engine.execute(&EngineInput::default()).await;

        assert!(matches!(
            result,
            Err(EngineError::ProcessFailed(message))
                if message == "engine execution is not expected in public API tests"
        ));
    }

    fn persona(id: &str) -> AgentPersonaProfile {
        AgentPersonaProfile {
            identity: AgentIdentity {
                id: AgentId::from_str(id),
                name: "Test persona".into(),
                description: String::new(),
                autonomy_level: AutonomyLevel::Supervised,
                created_at: chrono::Utc::now(),
            },
            role: AgentRole {
                role_code: "TEST".into(),
                title: "Tester".into(),
                primary_domain: "Testing".into(),
                capabilities: vec![],
            },
            soul: AgentSoul {
                persona: "Careful".into(),
                tone_of_voice: "Direct".into(),
                core_values: vec![],
                voice_profile: None,
            },
            mission: AgentMission {
                high_level_objective: "Test API behavior".into(),
                kpis: vec![],
                constraints: vec![],
            },
        }
    }

    #[test]
    fn with_persona_stores_the_last_profile_and_accepts_an_empty_profile() {
        let first = persona("first");
        let replacement = persona("");

        let configured = test_loop()
            .with_persona(first)
            .with_persona(replacement.clone());

        assert_eq!(configured.persona, Some(replacement));
    }

    #[tokio::test]
    async fn supplement_controller_clones_share_the_mailbox() -> Result<(), String> {
        let ooda = test_loop();
        let controller = ooda.supplement_controller();
        let another_clone = ooda.supplement_controller();

        assert!(another_clone.try_recv().await.is_none());
        controller
            .inject(crate::supplement::InFlightSupplement::new(
                "reviewer",
                "Keep the original objective",
            ))
            .await?;

        let received = another_clone
            .try_recv()
            .await
            .ok_or_else(|| "expected the injected supplement to be available".to_owned())?;
        assert_eq!(received.author_name, "reviewer");
        assert_eq!(received.content, "Keep the original objective");
        assert!(controller.try_recv().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn try_recv_reports_missing_supplement_without_panicking() {
        let ooda = test_loop();

        let missing = ooda.supplement_controller().try_recv().await;

        assert!(missing.is_none());
    }
}
