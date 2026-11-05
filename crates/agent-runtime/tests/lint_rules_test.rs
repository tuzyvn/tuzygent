//! Test suite verifying that agent-runtime adheres to strict lint rules,
//! specifically eliminating `ambiguous_glob_reexports` and ensuring clean, unambiguous public exports.

#![deny(ambiguous_glob_reexports)]

use agent_domain::GuardrailPolicy;
use agent_engine::cli::MockEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{
    // Core OODA components
    AutonomousOodaLoop, GoalDecomposer, QueueExecutionOutcome, SubtaskExecutor,
    MAX_SUBTASK_RETRIES, MAX_TOTAL_REMEDIATIONS,

    // Execution & Sandboxing
    ExecutionService, IsolationMode, SandboxConfig, SandboxDriverKind, SandboxEnvironment,

    // Workflow DAG
    WorkflowDefinition, WorkflowEdge, WorkflowNode, WorkflowOrchestrator, WorkflowRunReport,

    // Operator
    KernelOperator, RigExecutionSummary, RigSeatAssignment, SeatStatusReport,

    // Tasks & Verification
    SqliteTaskStore, TaskService, TaskStore, TaskVerifier,

    // Bus & RPC
    AgentSupervisor, InFlightSupplementController, InterAgentBus, WsRpcClient,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_unambiguous_public_api_exports() {
    let bus = InterAgentBus::new(16);
    let supervisor = AgentSupervisor::new(bus.clone());
    let agents = supervisor.list_agents().await;
    assert_eq!(agents.len(), 0);

    let sandbox_config = SandboxConfig::default();
    assert_eq!(sandbox_config.mode, IsolationMode::LocalDirectory);
    assert!(matches!(
        sandbox_config.driver,
        SandboxDriverKind::Host | SandboxDriverKind::Bubblewrap | SandboxDriverKind::Docker
    ));

    let env = SandboxEnvironment::new("/tmp", IsolationMode::LocalDirectory);
    assert!(env.worktree_path.is_none());

    let ctrl = InFlightSupplementController::new(10);
    assert!(ctrl
        .clone()
        .inject(agent_runtime::InFlightSupplement::new("admin", "test"))
        .await
        .is_ok());

    assert_eq!(MAX_SUBTASK_RETRIES, 3);
    assert_eq!(MAX_TOTAL_REMEDIATIONS, 6);
}

#[test]
fn test_no_module_collision_between_ooda_and_execution() {
    let exec_service = ExecutionService::new("/tmp", None, None);
    assert_eq!(exec_service.workspace_dir(), std::path::Path::new("/tmp"));

    let outcome = QueueExecutionOutcome::Completed;
    assert_eq!(outcome, QueueExecutionOutcome::Completed);

    let engine = Arc::new(MockEngine::new("mock output"));
    let decomposer = GoalDecomposer::new(
        engine,
        GuardrailPolicy::default_enterprise(),
        "/tmp",
        None,
        30,
    );
    let _ = decomposer;
}

#[test]
fn test_no_module_collision_between_operator_and_workflow_types() {
    let assignment = RigSeatAssignment {
        seat_id: "seat-1".into(),
        task: "task-1".into(),
        status: "Completed".into(),
        output: Some("done".into()),
    };
    let seat_status = SeatStatusReport {
        seat_id: "seat-1".into(),
        name: "Dev".into(),
        role: "dev".into(),
        engine: "mock".into(),
        harness: "tmux".into(),
        is_active: true,
        session_name: Some("seat-1".into()),
    };
    let report = RigExecutionSummary {
        rig_id: "rig-1".into(),
        rig_name: "test-rig".into(),
        operator_id: "op-1".into(),
        goal: "goal".into(),
        status: "Completed".into(),
        started_at: "now".into(),
        completed_at: "now".into(),
        assignments: vec![assignment],
        channel_dispatches: vec![],
        operator_synthesis: "all good".into(),
    };
    assert_eq!(report.rig_id, "rig-1");
    assert_eq!(seat_status.seat_id, "seat-1");

    let def = WorkflowDefinition {
        id: "wf-1".into(),
        name: "test-wf".into(),
        version: Some("1.0".into()),
        description: None,
        nodes: vec![],
        edges: vec![],
    };
    assert_eq!(def.id, "wf-1");

    let run_report = WorkflowRunReport {
        run_id: "run-1".into(),
        workflow_id: "wf-1".into(),
        workflow_name: "name".into(),
        workflow_version: "1.0".into(),
        triggered_by: "system".into(),
        trigger_type: "manual".into(),
        status: "Completed".into(),
        started_at: "now".into(),
        completed_at: "now".into(),
        duration_ms: 100,
        total_tokens_used: 120,
        node_results: Default::default(),
        timeline: vec![],
    };
    assert_eq!(run_report.workflow_id, "wf-1");
}

#[tokio::test]
async fn test_ooda_and_operator_initialization_coexistence() {
    let engine = Arc::new(MockEngine::new("mock output"));
    let temp_dir = std::env::temp_dir().join(format!("test_ooda_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let scratchpad = ScratchpadManager::new(temp_dir.to_str().unwrap());
    let durable = DurableMemoryStore::new(temp_dir.to_str().unwrap());

    let ooda = AutonomousOodaLoop::new(
        engine.clone(),
        GuardrailPolicy::default_enterprise(),
        scratchpad,
        durable,
        temp_dir.to_str().unwrap(),
    );
    let _ = ooda;

    let store: Arc<dyn TaskStore> = Arc::new(SqliteTaskStore::memory().unwrap());
    let task_service = TaskService::new(store.clone(), temp_dir.to_str().unwrap(), 2, None, None);
    let _ = task_service;

    let verifier = TaskVerifier::new(&temp_dir);
    let _ = verifier;

    let client = WsRpcClient::new(Duration::from_millis(50));
    assert_eq!(client.generation(), 0);

    let spec = agent_domain::RigSpec {
        rig: agent_domain::RigMetadata {
            id: "r1".into(),
            name: "r1".into(),
            description: None,
            version: Some("1.0".into()),
        },
        operator: agent_domain::RigOperatorConfig {
            agent_id: "op".into(),
            name: Some("Lead".into()),
            role: Some("Architect".into()),
            persona_path: None,
            engine: None,
            model: None,
        },
        seats: vec![],
        channels: vec![],
    };
    let op = KernelOperator::new(spec, temp_dir.to_str().unwrap());
    assert_eq!(op.workspace_dir, temp_dir.to_str().unwrap());

    let orchestrator = WorkflowOrchestrator::new(temp_dir.to_str().unwrap(), temp_dir.to_str().unwrap());
    let _ = orchestrator;

    let executor = SubtaskExecutor::new(
        engine,
        GuardrailPolicy::default_enterprise(),
        ScratchpadManager::new(temp_dir.to_str().unwrap()),
        DurableMemoryStore::new(temp_dir.to_str().unwrap()),
        temp_dir.to_str().unwrap(),
        InFlightSupplementController::new(5),
        None,
        None,
        60,
    );
    let _ = executor;

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// Verification module that wildcard re-exports from runtime, domain, and memory
/// do not trigger ambiguous_glob_reexports or conflict at compile time.
mod wildcard_export_validation {
    #![deny(ambiguous_glob_reexports)]
    pub use agent_domain::*;
    pub use agent_memory::*;
    pub use agent_runtime::*;

    #[test]
    fn test_wildcard_exports_are_unambiguous() {
        let _mode = IsolationMode::LocalDirectory;
        let _priority = Priority::Normal;
        let _guardrail = GuardrailPolicy::default_enterprise();
        let _store_type = std::any::type_name::<DurableMemoryStore>();
    }
}

/// TDD test case for TOPIC-20261104-89ca1e:
/// Permanently eliminates recurring mission failure in TOPIC-20261104-8bcc6a.
/// Verifies that all public types from `agent_runtime` are cleanly and unambiguously exported
/// at crate root and can be consumed together without any namespace collision or glob ambiguity.
#[test]
fn test_topic_89ca1e_root_cause_eliminated() {
    // 1. Core OODA & Decomposer types
    let _ = std::any::type_name::<AutonomousOodaLoop>();
    let _ = std::any::type_name::<GoalDecomposer>();
    let _ = std::any::type_name::<SubtaskExecutor>();
    let _ = std::any::type_name::<QueueExecutionOutcome>();
    assert_eq!(MAX_SUBTASK_RETRIES, 3);
    assert_eq!(MAX_TOTAL_REMEDIATIONS, 6);

    // 2. Execution & Sandbox types
    let _ = std::any::type_name::<ExecutionService>();
    let _ = std::any::type_name::<SandboxConfig>();
    let _ = std::any::type_name::<SandboxEnvironment>();
    let _ = std::any::type_name::<SandboxDriverKind>();
    let _ = std::any::type_name::<IsolationMode>();

    // 3. Workflow DAG & Orchestrator types
    let _ = std::any::type_name::<WorkflowOrchestrator>();
    let _ = std::any::type_name::<WorkflowDefinition>();
    let _ = std::any::type_name::<WorkflowRunReport>();
    let _ = std::any::type_name::<WorkflowNode>();
    let _ = std::any::type_name::<WorkflowEdge>();

    // 4. Operator types
    let _ = std::any::type_name::<KernelOperator>();
    let _ = std::any::type_name::<RigExecutionSummary>();
    let _ = std::any::type_name::<RigSeatAssignment>();
    let _ = std::any::type_name::<SeatStatusReport>();

    // 5. Tasks, Verifier & Storage types
    let _ = std::any::type_name::<TaskService>();
    let _ = std::any::type_name::<SqliteTaskStore>();
    let _ = std::any::type_name::<TaskVerifier>();

    // 6. Bus, Supervisor & WsRPC types
    let _ = std::any::type_name::<InterAgentBus>();
    let _ = std::any::type_name::<AgentSupervisor>();
    let _ = std::any::type_name::<InFlightSupplementController>();
    let _ = std::any::type_name::<WsRpcClient>();
}

/// TDD test case for TOPIC-20261104-c658ea:
/// Verifies that `runner_tests.rs` has been decomposed from a monolithic 478-line file
/// into modular, single-responsibility submodules under `ooda/execution/runner_tests/`,
/// with every submodule strictly under the 350-line P90 limit.
#[test]
fn test_topic_c658ea_runner_tests_modularization() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let old_monolithic_path = manifest_dir.join("src/ooda/execution/runner_tests.rs");
    assert!(
        !old_monolithic_path.is_file(),
        "Monolithic runner_tests.rs should no longer exist as a flat file"
    );

    let runner_tests_dir = manifest_dir.join("src/ooda/execution/runner_tests");
    assert!(
        runner_tests_dir.is_dir(),
        "runner_tests must be a modular directory"
    );

    let required_submodules = [
        "fixtures.rs",
        "lifecycle.rs",
        "guardrail.rs",
        "verification.rs",
        "mod.rs",
    ];

    for submodule in required_submodules {
        let submodule_path = runner_tests_dir.join(submodule);
        assert!(
            submodule_path.is_file(),
            "Required submodule {} must exist in runner_tests directory",
            submodule
        );

        let content = std::fs::read_to_string(&submodule_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", submodule_path.display()));
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "Submodule {} has {} lines, which exceeds P90 limit (350 lines)",
            submodule,
            line_count
        );
    }
}


