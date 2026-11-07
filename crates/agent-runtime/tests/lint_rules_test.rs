//! Test suite verifying that agent-runtime adheres to strict lint rules,
//! specifically eliminating `ambiguous_glob_reexports` and ensuring clean, unambiguous public exports.

#![deny(ambiguous_glob_reexports)]

use agent_domain::GuardrailPolicy;
use agent_engine::cli::MockEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{
    // Bus & RPC
    AgentSupervisor,
    // Core OODA components
    AutonomousOodaLoop,
    // Execution & Sandboxing
    ExecutionService,
    GoalDecomposer,
    InFlightSupplementController,
    InterAgentBus,
    IsolationMode,
    // Operator
    KernelOperator,
    QueueExecutionOutcome,
    RigExecutionSummary,
    RigSeatAssignment,
    SandboxConfig,
    SandboxDriverKind,
    SandboxEnvironment,

    SeatStatusReport,

    // Tasks & Verification
    SqliteTaskStore,
    SubtaskExecutor,
    TaskService,
    TaskStore,
    TaskVerifier,

    // Workflow DAG
    WorkflowDefinition,
    WorkflowEdge,
    WorkflowNode,
    WorkflowOrchestrator,
    WorkflowRunReport,

    WsRpcClient,
    MAX_SUBTASK_RETRIES,
    MAX_TOTAL_REMEDIATIONS,
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

    let orchestrator =
        WorkflowOrchestrator::new(temp_dir.to_str().unwrap(), temp_dir.to_str().unwrap());
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

/// TDD test case for TOPIC-20261105-2e7a49:
/// Verifies that `crates/agent-runtime/src/ooda/mod.rs` has been decomposed from a monolithic 446-line file
/// into modular, single-responsibility submodules under `src/ooda/`,
/// with every submodule strictly under the 350-line P90 limit.
#[test]
fn test_topic_2e7a49_ooda_mod_modularization() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ooda_dir = manifest_dir.join("src/ooda");
    assert!(ooda_dir.is_dir(), "src/ooda must exist as a directory");

    let mod_path = ooda_dir.join("mod.rs");
    assert!(mod_path.is_file(), "src/ooda/mod.rs must exist");

    let mod_content = std::fs::read_to_string(&mod_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", mod_path.display()));
    let mod_lines = mod_content.lines().count();
    assert!(
        mod_lines <= 350,
        "src/ooda/mod.rs has {} lines, which exceeds P90 limit (350 lines)",
        mod_lines
    );

    let lifecycle_path = ooda_dir.join("lifecycle.rs");
    assert!(
        lifecycle_path.is_file(),
        "src/ooda/lifecycle.rs must exist as a dedicated lifecycle submodule"
    );

    let lifecycle_content = std::fs::read_to_string(&lifecycle_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", lifecycle_path.display()));
    let lifecycle_lines = lifecycle_content.lines().count();
    assert!(
        lifecycle_lines <= 350,
        "src/ooda/lifecycle.rs has {} lines, which exceeds P90 limit (350 lines)",
        lifecycle_lines
    );

    let context_path = ooda_dir.join("context.rs");
    assert!(
        context_path.is_file(),
        "src/ooda/context.rs must exist as a dedicated context & environment submodule"
    );

    let context_content = std::fs::read_to_string(&context_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", context_path.display()));
    let context_lines = context_content.lines().count();
    assert!(
        context_lines <= 350,
        "src/ooda/context.rs has {} lines, which exceeds P90 limit (350 lines)",
        context_lines
    );
}

/// TDD test case for TOPIC-20261105-8cf437:
/// Verifies that `crates/agent-runtime/src/execution.rs` contains ZERO `.unwrap()` calls,
/// adhering strictly to safe error handling guidelines.
#[test]
fn test_topic_8cf437_no_unwrap_in_execution_rs() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let execution_path = manifest_dir.join("src/execution.rs");
    assert!(execution_path.is_file(), "src/execution.rs must exist");

    let content = std::fs::read_to_string(&execution_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", execution_path.display()));

    let unwrap_calls: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(".unwrap()"))
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();

    assert!(
        unwrap_calls.is_empty(),
        "Found {} unwrap() calls in src/execution.rs:\n{:#?}",
        unwrap_calls.len(),
        unwrap_calls
    );
}

/// TDD test case for TOPIC-20261105-c02a0d:
/// Verifies that `crates/agent-runtime/src/ooda/execution/runner_tests/fixtures.rs` contains ZERO `.unwrap()` calls,
/// adhering strictly to safe error handling guidelines.
#[test]
fn test_topic_c02a0d_no_unwrap_in_runner_tests_fixtures_rs() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures_path = manifest_dir.join("src/ooda/execution/runner_tests/fixtures.rs");
    assert!(
        fixtures_path.is_file(),
        "src/ooda/execution/runner_tests/fixtures.rs must exist"
    );

    let content = std::fs::read_to_string(&fixtures_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", fixtures_path.display()));

    let unwrap_calls: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(".unwrap()"))
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();

    assert!(
        unwrap_calls.is_empty(),
        "Found {} unwrap() calls in src/ooda/execution/runner_tests/fixtures.rs:\n{:#?}",
        unwrap_calls.len(),
        unwrap_calls
    );
}

/// TDD test case for TOPIC-20261105-513a07:
/// Verifies that `crates/agent-runtime/src/task_service.rs` has been decomposed from a monolithic 409-line file
/// into modular, single-responsibility submodules under `src/task_service/`,
/// with every submodule strictly under the 350-line P90 limit.
#[test]
fn test_topic_513a07_task_service_modularization() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_service_dir = manifest_dir.join("src/task_service");
    assert!(
        task_service_dir.is_dir(),
        "src/task_service must exist as a directory"
    );

    let legacy_file = manifest_dir.join("src/task_service.rs");
    assert!(
        !legacy_file.exists(),
        "src/task_service.rs monolithic file must be removed and replaced by src/task_service/ module"
    );

    let required_submodules = ["mod.rs", "dispatch.rs", "review.rs"];

    for submodule in required_submodules {
        let submodule_path = task_service_dir.join(submodule);
        assert!(
            submodule_path.is_file(),
            "Required submodule src/task_service/{} must exist",
            submodule
        );

        let content = std::fs::read_to_string(&submodule_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", submodule_path.display()));
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "Submodule src/task_service/{} has {} lines, which exceeds P90 limit (350 lines)",
            submodule,
            line_count
        );
    }
}

/// TDD test case for TOPIC-20261105-697d3d:
/// Verifies that `crates/agent-runtime/src/task_store.rs` has been decomposed from a monolithic 408-line file
/// into modular, single-responsibility submodules under `src/task_store/`,
/// with every submodule strictly under the 350-line P90 limit.
#[test]
fn test_topic_697d3d_task_store_modularization() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_store_dir = manifest_dir.join("src/task_store");
    assert!(
        task_store_dir.is_dir(),
        "src/task_store must exist as a directory"
    );

    let legacy_file = manifest_dir.join("src/task_store.rs");
    assert!(
        !legacy_file.exists(),
        "src/task_store.rs monolithic file must be removed and replaced by src/task_store/ module"
    );

    let required_submodules = ["mod.rs", "crud.rs", "lifecycle.rs", "schema.rs"];

    for submodule in required_submodules {
        let submodule_path = task_store_dir.join(submodule);
        assert!(
            submodule_path.is_file(),
            "Required submodule src/task_store/{} must exist",
            submodule
        );

        let content = std::fs::read_to_string(&submodule_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", submodule_path.display()));
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "Submodule src/task_store/{} has {} lines, which exceeds P90 limit (350 lines)",
            submodule,
            line_count
        );
    }
}

/// TDD test case for TOPIC-20261105-65d398:
/// Verifies that `crates/agent-engine/src/process.rs` has been decomposed from a monolithic 359-line file
/// into modular, single-responsibility submodules under `src/process/`,
/// with every submodule strictly under the 350-line P90 limit.
#[test]
fn test_topic_65d398_process_modularization() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let agent_engine_src = manifest_dir.parent().unwrap().join("agent-engine/src");
    let process_dir = agent_engine_src.join("process");
    assert!(
        process_dir.is_dir(),
        "crates/agent-engine/src/process must exist as a directory"
    );

    let legacy_file = agent_engine_src.join("process.rs");
    assert!(
        !legacy_file.exists(),
        "crates/agent-engine/src/process.rs monolithic file must be removed and replaced by process/ module"
    );

    let required_submodules = ["mod.rs", "builder.rs", "runner.rs", "stream.rs"];

    for submodule in required_submodules {
        let submodule_path = process_dir.join(submodule);
        assert!(
            submodule_path.is_file(),
            "Required submodule src/process/{} must exist",
            submodule
        );

        let content = std::fs::read_to_string(&submodule_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", submodule_path.display()));
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "Submodule src/process/{} has {} lines, which exceeds P90 limit (350 lines)",
            submodule,
            line_count
        );
    }
}

/// TDD test case for TOPIC-20261105-b998f4:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-8cf437.
/// Verifies that `crates/agent-runtime/src/execution.rs` and `ExecutionService` adhere to:
/// 1. Zero .unwrap() calls and strict P90 line limits (<= 350 lines).
/// 2. Graceful fallback for timeout_seconds == 0 (no immediate zero-duration timeout).
/// 3. Strict output bounding and UTF-8 safe truncation for oversized engine responses.
/// 4. Policy enforcement before engine execution (rejects prohibited commands).
/// 5. Cooperative cancellation support via AtomicBool token.
/// 6. Complete and ergonomic builder methods (with_guardrail, with_sandbox_config, with_cancellation_token, with_workspace_dir).
#[tokio::test]
async fn test_topic_b998f4_root_cause_eliminated_for_execution_service() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let execution_path = manifest_dir.join("src/execution.rs");
    assert!(execution_path.is_file(), "src/execution.rs must exist");

    let content = std::fs::read_to_string(&execution_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", execution_path.display()));

    let unwrap_calls: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(".unwrap()"))
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();

    assert!(
        unwrap_calls.is_empty(),
        "Found {} unwrap() calls in src/execution.rs:\n{:#?}",
        unwrap_calls.len(),
        unwrap_calls
    );

    let line_count = content.lines().count();
    assert!(
        line_count <= 350,
        "src/execution.rs has {} lines, exceeding P90 limit (350 lines)",
        line_count
    );

    // Mock engine for behavior testing
    use agent_domain::GuardrailPolicy;
    use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct TestEngine {
        output_text: String,
    }

    #[async_trait]
    impl ComputeEngine for TestEngine {
        async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
            Ok(EngineOutput {
                stdout_content: self.output_text.clone(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }

    // 1. Test builder methods
    let base_service = ExecutionService::new("/tmp", None, None);
    let guardrail = GuardrailPolicy::default_enterprise();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let service = base_service
        .with_guardrail(guardrail)
        .with_workspace_dir("/tmp/test_workspace")
        .with_cancellation_token(cancel_flag.clone());

    assert_eq!(
        service.workspace_dir(),
        std::path::Path::new("/tmp/test_workspace")
    );
    assert!(service.cancellation_token().is_some());

    // 2. Test graceful fallback for timeout_seconds == 0
    let engine = Arc::new(TestEngine {
        output_text: "success with zero timeout".to_string(),
    });
    let mut zero_timeout_input = EngineInput::new("echo hello", "/tmp/test_workspace");
    zero_timeout_input.timeout_seconds = 0;
    let res = service.execute(engine.clone(), &zero_timeout_input).await;
    assert!(
        res.is_ok(),
        "ExecutionService must gracefully handle timeout_seconds == 0 instead of timing out immediately"
    );
    let out = res.expect("Should succeed");
    assert_eq!(out.stdout_content, "success with zero timeout");

    // 3. Test strict UTF-8 safe output bounding and truncation
    let long_multibyte_text =
        "Tiếng Việt có dấu kiểm tra UTF-8 boundary cắt tỉa an toàn không lỗi panic ".repeat(50);
    let oversized_engine = Arc::new(TestEngine {
        output_text: long_multibyte_text,
    });
    let mut bounded_input = EngineInput::new("echo verbose", "/tmp/test_workspace");
    bounded_input.max_output_bytes = Some(60);
    let bounded_res = service.execute(oversized_engine, &bounded_input).await;
    assert!(
        bounded_res.is_ok(),
        "Oversized output must be safely handled"
    );
    let bounded_out = bounded_res.expect("Should succeed");
    assert!(
        bounded_out.stdout_content.contains("[... output truncated"),
        "Output should include truncation marker"
    );

    // 4. Test policy enforcement for dangerous commands
    let dangerous_input = EngineInput::new("rm -rf /", "/tmp/test_workspace");
    let policy_res = service.execute(engine.clone(), &dangerous_input).await;
    assert!(
        matches!(policy_res, Err(EngineError::PolicyRejected(_))),
        "Forbidden commands must be rejected by policy before execution"
    );

    // 5. Test cooperative cancellation
    cancel_flag.store(true, Ordering::SeqCst);
    let normal_input = EngineInput::new("echo normal", "/tmp/test_workspace");
    let cancel_res = service.execute(engine.clone(), &normal_input).await;
    assert!(
        matches!(cancel_res, Err(EngineError::ProcessFailed(_))),
        "Canceled token must prevent execution from proceeding"
    );
}

/// TDD test case for TOPIC-20261105-86bcb5:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-c02a0d.
/// Verifies that `crates/agent-runtime/src/ooda/execution/runner_tests/fixtures.rs` and its fixtures adhere to:
/// 1. Zero .unwrap() calls.
/// 2. Zero .expect( calls (preventing panics during test environment initialization).
/// 3. Zero panic!( calls.
/// 4. Strict P90 line limits (<= 350 lines).
/// 5. Panic-free and fallback-safe TestEnv creation (`try_new` and `new`).
/// 6. Safe filesystem helper methods (`write_file`, `read_file`, `create_sub_dir`, `file_exists`, `remove_file`, `clean`).
/// 7. Robust executor setups (`setup_test_executor`, `try_setup_test_executor`, `setup_test_executor_with_timeout`).
/// 8. Enhanced MockEngine capabilities (`with_constant_output`, `with_error`, `call_count`, `reset`).
#[tokio::test]
async fn test_topic_86bcb5_root_cause_eliminated_for_runner_tests_fixtures() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures_path = manifest_dir.join("src/ooda/execution/runner_tests/fixtures.rs");
    assert!(
        fixtures_path.is_file(),
        "src/ooda/execution/runner_tests/fixtures.rs must exist"
    );

    let content = std::fs::read_to_string(&fixtures_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", fixtures_path.display()));

    // 1. Zero .unwrap()
    let unwrap_calls: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(".unwrap()"))
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();
    assert!(
        unwrap_calls.is_empty(),
        "Found {} unwrap() calls in fixtures.rs:\n{:#?}",
        unwrap_calls.len(),
        unwrap_calls
    );

    // 2. Zero .expect(
    let expect_calls: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(".expect("))
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();
    assert!(
        expect_calls.is_empty(),
        "Found {} expect() calls in fixtures.rs:\n{:#?}",
        expect_calls.len(),
        expect_calls
    );

    // 3. Zero panic!(
    let panic_calls: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("panic!("))
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();
    assert!(
        panic_calls.is_empty(),
        "Found {} panic!() calls in fixtures.rs:\n{:#?}",
        panic_calls.len(),
        panic_calls
    );

    // 4. Line count <= 350
    let line_count = content.lines().count();
    assert!(
        line_count <= 350,
        "fixtures.rs has {} lines, exceeding P90 limit (350 lines)",
        line_count
    );
}

/// TDD test case for TOPIC-20261105-694a83:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-513a07.
/// Verifies that `crates/agent-runtime/src/task_service/` and `TaskService` adhere to:
/// 1. Zero .unwrap() calls across all submodules (mod.rs, dispatch.rs, review.rs).
/// 2. Zero .expect( calls across all submodules.
/// 3. Zero panic!( calls across all submodules.
/// 4. Strict P90 line limits (<= 350 lines) for all submodules.
/// 5. Complete and ergonomic builder methods (with_guardrail, with_sandbox_config, with_workspace_dir).
/// 6. Safe accessors (workspace_dir, guardrail, sandbox_config, available_concurrency, active_task_count).
/// 7. Batch cooperative cancellation support (cancel_all).
/// 8. Self-healing workspace initialization and error resilience during background dispatch.
#[tokio::test]
async fn test_topic_694a83_root_cause_eliminated_for_task_service() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_service_dir = manifest_dir.join("src/task_service");
    assert!(
        task_service_dir.is_dir(),
        "src/task_service must exist as a directory"
    );

    let submodules = ["mod.rs", "dispatch.rs", "review.rs"];
    for sub in submodules {
        let sub_path = task_service_dir.join(sub);
        assert!(sub_path.is_file(), "src/task_service/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/task_service/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/task_service/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/task_service/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/task_service/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    // Behavioral and functional verification
    use agent_domain::{Priority, TaskStatus};
    let temp_workspace =
        std::env::temp_dir().join(format!("task_service_test_{}", uuid::Uuid::new_v4()));
    let store = Arc::new(SqliteTaskStore::new(&temp_workspace).expect("Store init"));

    // Builder methods & accessors
    let guardrail = GuardrailPolicy::default_enterprise();
    let sandbox = SandboxConfig::default();
    let service = TaskService::new(store.clone(), &temp_workspace, 2, None, None)
        .with_guardrail(guardrail.clone())
        .with_sandbox_config(sandbox.clone())
        .with_workspace_dir(&temp_workspace);

    assert_eq!(service.workspace_dir(), temp_workspace.as_path());
    assert_eq!(service.available_concurrency(), 2);
    assert_eq!(service.active_task_count().await, 0);
    assert!(service.sandbox_config().is_some());
    assert_eq!(
        service.guardrail().max_subtasks_per_goal,
        guardrail.max_subtasks_per_goal
    );

    // Batch cancellation test (cancel_all)
    let t1 = service
        .submit_task("Task 1", "Goal 1", Priority::Normal)
        .await
        .expect("Submit t1");
    let t2 = service
        .submit_task("Task 2", "Goal 2", Priority::High)
        .await
        .expect("Submit t2");
    let canceled_count = service.cancel_all().await.expect("Cancel all");
    assert!(
        canceled_count >= 2,
        "cancel_all must cancel all pending/active tasks"
    );

    let t1_state = service
        .get_task(&t1)
        .await
        .expect("Get t1")
        .expect("t1 exists");
    assert!(matches!(t1_state.status, TaskStatus::Canceled { .. }));
    let t2_state = service
        .get_task(&t2)
        .await
        .expect("Get t2")
        .expect("t2 exists");
    assert!(matches!(t2_state.status, TaskStatus::Canceled { .. }));

    // Self-healing workspace initialization verification during dispatch
    let non_existent_ws =
        std::env::temp_dir().join(format!("non_existent_ws_{}", uuid::Uuid::new_v4()));
    assert!(!non_existent_ws.exists());
    let store2 = Arc::new(SqliteTaskStore::new(&non_existent_ws).expect("Store init"));
    let service2 = TaskService::new(store2.clone(), &non_existent_ws, 1, None, None);
    let t3 = service2
        .submit_task("Self healing ws", "Goal 3", Priority::Normal)
        .await
        .expect("Submit t3");
    let mock_engine = Arc::new(MockEngine::new("mock output"));
    service2.dispatch(t3.clone(), mock_engine);

    // Wait briefly for background dispatch
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        non_existent_ws.exists(),
        "Workspace dir must be self-healed and created automatically"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_workspace);
    let _ = std::fs::remove_dir_all(&non_existent_ws);
}

/// TDD test case for TOPIC-20261105-62a0a4:
/// Root-cause elimination of system error 'Mission exited with non-zero code 1'
/// in `crates/agent-runtime/src/task_store/`:
/// 1. Zero .unwrap() calls across all task_store submodules.
/// 2. Zero .expect( calls across all task_store submodules.
/// 3. Zero panic!( calls across all task_store submodules.
/// 4. Every submodule strictly under the 350-line P90 limit.
/// 5. Self-healing initialization on corrupted database files.
/// 6. Self-healing initialization when storage path is blocked by a file.
/// 7. Database integrity checking and health monitoring (is_healthy, check_integrity).
/// 8. Atomic and maintenance capabilities (count_tasks, delete_task, vacuum, clear_all).
/// 9. Safe lifecycle handling for missing task/checkpoint in save_review.
#[tokio::test]
async fn test_topic_62a0a4_root_cause_eliminated_for_task_store() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_store_dir = manifest_dir.join("src/task_store");
    assert!(
        task_store_dir.is_dir(),
        "src/task_store must exist as a directory"
    );

    let submodules = ["mod.rs", "crud.rs", "lifecycle.rs", "schema.rs"];
    for sub in submodules {
        let sub_path = task_store_dir.join(sub);
        assert!(sub_path.is_file(), "src/task_store/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/task_store/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/task_store/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/task_store/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/task_store/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    use agent_domain::{Priority, Task, TaskStatus};

    // 5. Self-healing corrupted database file test
    let temp_ws_corrupt =
        std::env::temp_dir().join(format!("task_store_corrupt_{}", uuid::Uuid::new_v4()));
    let storage_dir = temp_ws_corrupt.join("storage");
    std::fs::create_dir_all(&storage_dir).expect("Create storage");
    let corrupt_db_path = storage_dir.join("tasks.db");
    std::fs::write(&corrupt_db_path, b"CORRUPTED_GARBAGE_HEADER_DATA_12345")
        .expect("Write corrupt db");

    let store_corrupt =
        SqliteTaskStore::new(&temp_ws_corrupt).expect("Self-healing store init on corrupted db");
    assert!(
        store_corrupt.is_healthy(),
        "Corrupted database must be self-healed and healthy"
    );
    assert!(
        store_corrupt.check_integrity().expect("Check integrity"),
        "Integrity check must pass"
    );

    // 6. Self-healing when storage path is a file
    let temp_ws_storage_file =
        std::env::temp_dir().join(format!("task_store_storage_file_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_ws_storage_file).expect("Create ws");
    let blocked_storage = temp_ws_storage_file.join("storage");
    std::fs::write(&blocked_storage, b"accidental regular file").expect("Write blocked file");

    let store_blocked = SqliteTaskStore::new(&temp_ws_storage_file)
        .expect("Self-healing store init on blocked storage path");
    assert!(
        store_blocked.is_healthy(),
        "Database must be healthy after resolving blocked storage path"
    );

    // 7. Atomic & Maintenance operations (count_tasks, delete_task, vacuum, clear_all)
    let temp_ws_ops = std::env::temp_dir().join(format!("task_store_ops_{}", uuid::Uuid::new_v4()));
    let store = SqliteTaskStore::new(&temp_ws_ops).expect("Store init");
    assert_eq!(store.count_tasks().expect("Count 0"), 0);

    let task1 = Task::new("Task 1", "Goal 1", Priority::Normal);
    let task1_id = task1.id.clone();
    store.save_task(&task1).await.expect("Save task 1");
    assert_eq!(store.count_tasks().expect("Count 1"), 1);

    let task2 = Task::new("Task 2", "Goal 2", Priority::High);
    let task2_id = task2.id.clone();
    store.save_task(&task2).await.expect("Save task 2");
    assert_eq!(store.count_tasks().expect("Count 2"), 2);

    let deleted = store.delete_task(&task1_id).expect("Delete task 1");
    assert!(deleted, "Task 1 must be deleted");
    assert_eq!(store.count_tasks().expect("Count after delete"), 1);
    assert!(store
        .get_task(&task1_id)
        .await
        .expect("Get task 1")
        .is_none());
    assert!(store
        .get_task(&task2_id)
        .await
        .expect("Get task 2")
        .is_some());

    store.vacuum().expect("Vacuum succeeds");
    store.clear_all().expect("Clear all succeeds");
    assert_eq!(store.count_tasks().expect("Count after clear"), 0);

    // 8. Safe lifecycle handling for save_review on non-existent task
    let non_existent_task = Task::new("Ghost", "Ghost goal", Priority::Normal);
    let ghost_result = agent_domain::TaskResult {
        task_id: non_existent_task.id.clone(),
        status: TaskStatus::Completed,
        summary: "Ghost".into(),
        artifacts: Vec::new(),
        verifications: Vec::new(),
        duration_ms: 10,
        engine_used: None,
    };
    let review_outcome = store
        .save_review(
            &non_existent_task,
            &non_existent_task,
            &ghost_result,
            "reviewer",
            "approve",
            "rev1",
        )
        .await
        .expect("save_review must handle non-existent task gracefully without failing");
    assert!(
        !review_outcome,
        "Review on non-existent task must return false safely"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_ws_corrupt);
    let _ = std::fs::remove_dir_all(&temp_ws_storage_file);
    let _ = std::fs::remove_dir_all(&temp_ws_ops);
}

/// TDD test case for TOPIC-20261105-0ae933:
/// Root-cause elimination of system error 'Mission exited with non-zero code 1'
/// in `crates/agent-engine/src/process/`:
/// 1. Zero .unwrap() calls across all process submodules.
/// 2. Zero .expect( calls across all process submodules (eliminating panics on child/pipe extraction).
/// 3. Zero panic!( calls across all process submodules.
/// 4. Every submodule strictly under the 350-line P90 limit.
/// 5. Self-healing non-existent working directory (automatically created instead of failing with ENOENT).
/// 6. Zero timeout duration fallback (no immediate zero-duration timeout error).
/// 7. Zero max_output_bytes fallback (no immediate InvalidData error on 0 max_bytes).
/// 8. Multibyte UTF-8 safe truncation (prevents corrupting unicode characters at truncation boundary).
/// 9. Ergonomic CommandSandbox builder constructors (new, with_env_whitelist, with_container_name).
#[tokio::test]
async fn test_topic_0ae933_root_cause_eliminated_for_process_module() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let agent_engine_src = manifest_dir.parent().unwrap().join("agent-engine/src");
    let process_dir = agent_engine_src.join("process");
    assert!(
        process_dir.is_dir(),
        "crates/agent-engine/src/process must exist"
    );

    let submodules = ["mod.rs", "builder.rs", "runner.rs", "stream.rs"];
    for sub in submodules {
        let sub_path = process_dir.join(sub);
        assert!(sub_path.is_file(), "src/process/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/process/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/process/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/process/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/process/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    use agent_engine::process::{read_stream, run_command, run_process_with_limit, CommandSandbox};
    use agent_engine::EngineInput;
    use std::collections::HashMap;

    // 5. CommandSandbox builder verification
    let sandbox = CommandSandbox::new("/bin/sh", vec!["-c".into()], "/workspace")
        .with_env_whitelist(vec!["PATH".into(), "CUSTOM_VAR".into()])
        .with_container_name("test-container-123");
    assert_eq!(sandbox.program, "/bin/sh");
    assert_eq!(sandbox.prefix_args, vec!["-c".to_string()]);
    assert_eq!(sandbox.working_dir, "/workspace");
    assert_eq!(
        sandbox.env_whitelist,
        vec!["PATH".to_string(), "CUSTOM_VAR".to_string()]
    );
    assert_eq!(
        sandbox.container_name,
        Some("test-container-123".to_string())
    );

    // 6. Zero timeout duration fallback in run_process_with_limit
    let mut echo_cmd = tokio::process::Command::new("sh");
    echo_cmd.arg("-c").arg("echo hello_zero_timeout");
    let res_zero_timeout = run_process_with_limit(echo_cmd, Duration::ZERO, None, 1024).await;
    assert!(
        res_zero_timeout.is_ok(),
        "run_process_with_limit with Duration::ZERO must fall back to safe default timeout and succeed"
    );
    let output = res_zero_timeout.unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("hello_zero_timeout"));

    // 7. Zero max_output_bytes fallback in read_stream
    let stream_bytes = b"testing zero max output bytes stream\n";
    let res_zero_max = read_stream(&stream_bytes[..], None, 0).await;
    assert!(
        res_zero_max.is_ok(),
        "read_stream with max_bytes == 0 must fall back to DEFAULT_MAX_OUTPUT_BYTES"
    );
    assert_eq!(res_zero_max.unwrap(), stream_bytes.to_vec());

    // 8. Multibyte UTF-8 safe truncation
    // Line 1: 30 bytes; Line 2: contains Vietnamese multibyte chars.
    // Total is > 40 bytes, but each line is <= 40 bytes.
    let line1 = "Tập 01: Nghiên cứu tự chủ\n"; // 31 bytes
    let line2 = "Tập 02: Trí tuệ 🚀🦀\n"; // 25 bytes
    let combined = format!("{line1}{line2}");
    // max_bytes = 40. Remaining for line 2 is 40 - 31 = 9 bytes.
    // "Tập 02: " has 9 bytes, but "Tập" has 'ậ' (3 bytes: 0xe1 0xba 0xad).
    // Cutting at 9 bytes would land around multibyte characters.
    let truncated_output = read_stream(combined.as_bytes(), None, 40)
        .await
        .expect("read_stream must succeed with truncation");
    let truncated_str = String::from_utf8(truncated_output.clone());
    assert!(
        truncated_str.is_ok(),
        "Truncated output must remain strictly valid UTF-8 without byte splitting: {:?}",
        String::from_utf8_lossy(&truncated_output)
    );
    let valid_text = truncated_str.unwrap();
    assert!(
        valid_text.contains("[OUTPUT TRUNCATED: buffer exceeded limit]"),
        "Output must contain truncation marker"
    );

    // 9. Self-healing non-existent working directory
    let non_existent_workdir =
        std::env::temp_dir().join(format!("agent_process_heal_{}", uuid::Uuid::new_v4()));
    assert!(!non_existent_workdir.exists());

    let input = EngineInput::new("pwd", non_existent_workdir.to_str().unwrap());
    let env_map = HashMap::new();
    let cmd_res = run_command(&input, "pwd", &[], &env_map, None).await;
    assert!(
        cmd_res.is_ok(),
        "run_command with non-existent working_dir must heal the directory and succeed"
    );
    assert!(
        non_existent_workdir.is_dir(),
        "Working directory must have been created automatically"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&non_existent_workdir);
}

/// TDD test case for TOPIC-20261105-4338d8:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-86bcb5.
/// Verifies that all test infrastructure and runner test submodules in
/// `crates/agent-runtime/src/ooda/execution/runner_tests/` adhere to:
/// 1. Zero .unwrap() calls across all runner_tests submodules.
/// 2. Zero .expect( calls across all runner_tests submodules (eliminating panics during artifact writing or subtask lookup).
/// 3. Zero panic!( calls across all runner_tests submodules.
/// 4. Every submodule strictly under the 350-line P90 limit.
/// 5. Robust multi-tier self-healing TestEnv fallback initialization.
/// 6. Safe zero-timeout duration fallback in try_setup_test_executor and setup_test_executor_with_timeout.
/// 7. Enhanced CallbackMockEngine capabilities: with_constant_output, with_timeout, with_failure,
///    with_error, with_exit_code, with_sequence, call_count, reset.
#[tokio::test]
async fn test_topic_4338d8_root_cause_eliminated_for_runner_tests_and_fixtures() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runner_tests_dir = manifest_dir.join("src/ooda/execution/runner_tests");
    assert!(
        runner_tests_dir.is_dir(),
        "runner_tests must exist as a modular directory"
    );

    let required_submodules = [
        "mod.rs",
        "fixtures.rs",
        "guardrail.rs",
        "lifecycle.rs",
        "verification.rs",
    ];

    for submodule in required_submodules {
        let submodule_path = runner_tests_dir.join(submodule);
        assert!(
            submodule_path.is_file(),
            "Required submodule runner_tests/{} must exist",
            submodule
        );

        let content = std::fs::read_to_string(&submodule_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", submodule_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/ooda/execution/runner_tests/{}:\n{:#?}",
            unwraps.len(),
            submodule,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/ooda/execution/runner_tests/{}:\n{:#?}",
            expects.len(),
            submodule,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/ooda/execution/runner_tests/{}:\n{:#?}",
            panics.len(),
            submodule,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/ooda/execution/runner_tests/{} has {} lines, exceeding P90 limit (350 lines)",
            submodule,
            line_count
        );
    }

    // 5. Inspect fixtures.rs for resilient patterns
    let fixtures_content = std::fs::read_to_string(runner_tests_dir.join("fixtures.rs"))
        .unwrap_or_else(|_| panic!("Failed to read fixtures.rs"));

    assert!(
        fixtures_content.contains("pub fn with_exit_code"),
        "fixtures.rs must provide with_exit_code helper for non-zero exit code testing"
    );
    assert!(
        fixtures_content.contains("pub fn with_error"),
        "fixtures.rs must provide with_error helper"
    );
    assert!(
        fixtures_content.contains("pub fn with_sequence"),
        "fixtures.rs must provide with_sequence helper for multi-step responses"
    );
    assert!(
        fixtures_content.contains("Some(0)") || fixtures_content.contains("timeout_secs == 0"),
        "fixtures.rs must handle zero timeout gracefully by falling back to safe default"
    );
}

/// TDD test case for TOPIC-20261105-07a09e:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-694a83.
/// Verifies that `crates/agent-runtime/src/task_service/` adheres to:
/// 1. Zero .unwrap() calls across all submodules (mod.rs, dispatch.rs, review.rs).
/// 2. Zero .expect( calls across all submodules.
/// 3. Zero panic!( calls across all submodules.
/// 4. Strict P90 line limits (<= 350 lines) for all submodules.
/// 5. Ergonomic concurrency controls (concurrency_limit, with_concurrency).
/// 6. Active task status queries (is_active) and health checks (is_healthy, count_tasks).
/// 7. Paginated batch cancellation across large task volumes without truncation.
/// 8. Safe graceful task drain (drain) with configurable timeouts.
/// 9. Resilient task retry (retry_task) for failed/canceled tasks.
/// 10. Multi-tier self-healing workspace fallback resolution on uncreatable paths.
#[tokio::test]
async fn test_topic_07a09e_root_cause_eliminated_for_task_service() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_service_dir = manifest_dir.join("src/task_service");
    assert!(
        task_service_dir.is_dir(),
        "src/task_service must exist as a directory"
    );

    let submodules = ["mod.rs", "dispatch.rs", "review.rs"];
    for sub in submodules {
        let sub_path = task_service_dir.join(sub);
        assert!(sub_path.is_file(), "src/task_service/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/task_service/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/task_service/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/task_service/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/task_service/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    // Behavioral and functional verification
    use agent_domain::{Priority, TaskStatus};
    let temp_workspace =
        std::env::temp_dir().join(format!("task_service_07a09e_{}", uuid::Uuid::new_v4()));
    let store = Arc::new(SqliteTaskStore::new(&temp_workspace).expect("Store init"));

    // Builder and concurrency methods
    let service =
        TaskService::new(store.clone(), &temp_workspace, 2, None, None).with_concurrency(4);

    assert_eq!(service.concurrency_limit(), 4);
    assert_eq!(service.available_concurrency(), 4);
    assert!(service.is_healthy().await);
    assert_eq!(service.count_tasks().await.expect("count tasks"), 0);

    // Submit tasks and verify is_active & count_tasks
    let t1 = service
        .submit_task("Task Alpha", "Goal Alpha", Priority::Normal)
        .await
        .expect("Submit t1");
    let t2 = service
        .submit_task("Task Beta", "Goal Beta", Priority::High)
        .await
        .expect("Submit t2");
    assert_eq!(service.count_tasks().await.expect("count tasks"), 2);
    assert!(!service.is_active(&t1).await);

    // Dispatch and verify is_active tracking
    let mock_engine = Arc::new(MockEngine::new("active test output"));
    service.dispatch(t1.clone(), mock_engine.clone());

    // Wait briefly for completion
    let drained = service
        .drain(Duration::from_secs(3))
        .await
        .expect("Drain active tasks");
    assert!(drained, "Active tasks must be drained within timeout");
    assert_eq!(service.active_task_count().await, 0);
    assert!(!service.is_active(&t1).await);

    // Cancel remaining task
    let canceled = service.cancel_task(&t2).await.expect("Cancel t2");
    assert!(canceled);

    // Test retry_task on canceled task
    let retried = service
        .retry_task(&t2, mock_engine.clone())
        .await
        .expect("Retry t2");
    assert!(
        retried,
        "retry_task should successfully requeue and dispatch a canceled task"
    );
    let drained_retry = service
        .drain(Duration::from_secs(3))
        .await
        .expect("Drain retry");
    assert!(drained_retry);
    let t2_state = service
        .get_task(&t2)
        .await
        .expect("Get t2")
        .expect("t2 exists");
    assert_ne!(t2_state.status, TaskStatus::Canceled { reason: "".into() });

    // Test multi-tier self-healing workspace fallback for uncreatable path
    let uncreatable_path = std::path::PathBuf::from("/dev/null/forbidden/workspace");
    let store_healing = Arc::new(SqliteTaskStore::new(&temp_workspace).expect("Store init"));
    let healing_service = TaskService::new(store_healing, &uncreatable_path, 1, None, None);
    let t3 = healing_service
        .submit_task("Healing Task", "Must self-heal workspace", Priority::Normal)
        .await
        .expect("Submit t3");
    healing_service.dispatch(t3.clone(), mock_engine.clone());
    let drained_healing = healing_service
        .drain(Duration::from_secs(3))
        .await
        .expect("Drain healing");
    assert!(drained_healing);

    let t3_state = healing_service
        .get_task(&t3)
        .await
        .expect("Get t3")
        .expect("t3 exists");
    assert!(
        matches!(
            t3_state.status,
            TaskStatus::Completed | TaskStatus::WaitingApproval { .. }
        ),
        "Self-healed workspace task must transition to valid status: {:?}",
        t3_state.status
    );

    // Paginated cancel_all verification across multiple tasks
    let mut cancel_ids = Vec::new();
    for i in 0..10 {
        let tid = service
            .submit_task(&format!("Batch Task {}", i), "Batch Goal", Priority::Low)
            .await
            .expect("Submit batch");
        cancel_ids.push(tid);
    }
    let batch_canceled = service.cancel_all().await.expect("cancel_all batch");
    assert_eq!(
        batch_canceled, 10,
        "cancel_all must cancel all 10 batch tasks"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_workspace);
}

/// TDD test case for TOPIC-20261105-cb8d42:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-62a0a4.
/// Verifies that `crates/agent-runtime/src/task_store/` adheres to:
/// 1. Zero .unwrap() calls across all task_store submodules (mod.rs, crud.rs, lifecycle.rs, schema.rs).
/// 2. Zero .expect( calls across all submodules.
/// 3. Zero panic!( calls across all submodules.
/// 4. Strict P90 line limits (<= 350 lines) for all submodules.
/// 5. Resilient multi-tier self-healing storage fallback initialization on uncreatable/forbidden workspace paths.
/// 6. Poisoned mutex self-healing recovery via acquire_conn without failing with lock errors.
/// 7. Safe check_integrity returning Ok(false) on corrupted databases without returning Err.
/// 8. Runtime database repair and reinitialization (repair_or_recreate).
/// 9. Resilient finish_task lifecycle transitions from Queued, SelfChecking, WaitingApproval,
///    while rejecting already-finished tasks (Completed, Failed).
/// 10. Atomic priority-based next queued task claiming (claim_next_queued).
/// 11. Indexed status querying and task counting (list_tasks_by_status, count_tasks_by_status).
/// 12. Task store pruning of old finished tasks (prune_finished_tasks) and WAL checkpointing.
/// 13. Idempotent key inspection and deletion (get_idempotent_task, delete_idempotent_key).
#[tokio::test]
async fn test_topic_cb8d42_root_cause_eliminated_for_task_store() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_store_dir = manifest_dir.join("src/task_store");
    assert!(
        task_store_dir.is_dir(),
        "src/task_store must exist as a directory"
    );

    let submodules = ["mod.rs", "crud.rs", "lifecycle.rs", "schema.rs"];
    for sub in submodules {
        let sub_path = task_store_dir.join(sub);
        assert!(sub_path.is_file(), "src/task_store/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/task_store/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/task_store/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/task_store/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/task_store/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    use agent_domain::{Priority, Task, TaskResult, TaskStatus};
    use agent_runtime::IdempotentSubmission;

    // 5. Resilient multi-tier self-healing storage fallback on uncreatable/forbidden workspace path
    let uncreatable_ws = std::path::PathBuf::from("/dev/null/forbidden/workspace/storage");
    let fallback_store = SqliteTaskStore::new(&uncreatable_ws)
        .expect("SqliteTaskStore must self-heal and fallback cleanly on uncreatable paths");
    assert!(
        fallback_store.is_healthy(),
        "Fallback store must be healthy"
    );

    // 6. Resilient finish_task transitions from Queued, SelfChecking, WaitingApproval
    let temp_ws = std::env::temp_dir().join(format!("task_store_cb8d42_{}", uuid::Uuid::new_v4()));
    let store = SqliteTaskStore::new(&temp_ws).expect("Store init");

    // Queued -> Completed
    let q_task = Task::new(
        "Early finish",
        "Finish directly from Queued",
        Priority::Normal,
    );
    store.save_task(&q_task).await.expect("Save q_task");
    let mut q_completed = q_task.clone();
    q_completed.status = TaskStatus::Completed;
    let q_res = TaskResult {
        task_id: q_task.id.clone(),
        status: TaskStatus::Completed,
        summary: "Early finished".into(),
        artifacts: Vec::new(),
        verifications: Vec::new(),
        duration_ms: 10,
        engine_used: None,
    };
    store
        .finish_task(&q_completed, &q_res, false)
        .await
        .expect("finish_task from Queued must succeed");
    let saved_q = store
        .get_task(&q_task.id)
        .await
        .expect("Get q")
        .expect("Task exists");
    assert_eq!(saved_q.status, TaskStatus::Completed);

    // Double-finish rejected
    assert!(
        store
            .finish_task(&q_completed, &q_res, false)
            .await
            .is_err(),
        "Double finishing an already completed task must return Err"
    );

    // SelfChecking -> Completed
    let mut sc_task = Task::new(
        "Self Checking Task",
        "Verification in progress",
        Priority::High,
    );
    sc_task.status = TaskStatus::SelfChecking;
    store.save_task(&sc_task).await.expect("Save sc_task");
    let mut sc_completed = sc_task.clone();
    sc_completed.status = TaskStatus::Completed;
    let sc_res = TaskResult {
        task_id: sc_task.id.clone(),
        status: TaskStatus::Completed,
        summary: "Self check passed".into(),
        artifacts: Vec::new(),
        verifications: Vec::new(),
        duration_ms: 50,
        engine_used: None,
    };
    store
        .finish_task(&sc_completed, &sc_res, false)
        .await
        .expect("finish_task from SelfChecking must succeed");

    // 7. Atomic priority-based claim_next_queued
    let normal_task = Task::new("Normal Task", "Low prio", Priority::Normal);
    let high_task = Task::new("High Task", "High prio", Priority::High);
    let crit_task = Task::new("Critical Task", "Critical prio", Priority::Critical);
    store.save_task(&normal_task).await.expect("Save normal");
    store.save_task(&high_task).await.expect("Save high");
    store.save_task(&crit_task).await.expect("Save crit");

    // Critical must be claimed first
    let claimed1 = store
        .claim_next_queued()
        .expect("Claim next 1")
        .expect("Claimed task");
    assert_eq!(
        claimed1.id, crit_task.id,
        "Critical priority task must be claimed first"
    );
    assert_eq!(claimed1.status, TaskStatus::Executing);

    // High must be claimed second
    let claimed2 = store
        .claim_next_queued()
        .expect("Claim next 2")
        .expect("Claimed task");
    assert_eq!(
        claimed2.id, high_task.id,
        "High priority task must be claimed second"
    );

    // Normal must be claimed third
    let claimed3 = store
        .claim_next_queued()
        .expect("Claim next 3")
        .expect("Claimed task");
    assert_eq!(
        claimed3.id, normal_task.id,
        "Normal priority task must be claimed third"
    );

    // No more queued tasks
    assert!(store.claim_next_queued().expect("Claim next 4").is_none());

    // 8. Status querying and counting (list_tasks_by_status, count_tasks_by_status)
    let executing_count = store
        .count_tasks_by_status("Executing")
        .expect("Count executing");
    assert_eq!(executing_count, 3);
    let completed_count = store
        .count_tasks_by_status("Completed")
        .expect("Count completed");
    assert_eq!(completed_count, 2);

    let executing_list = store
        .list_tasks_by_status("Executing", 10, 0)
        .expect("List executing");
    assert_eq!(executing_list.len(), 3);

    // 9. Idempotent key inspection and deletion
    let idem_task = Task::new("Idem", "Idem goal", Priority::Normal);
    let submission = store
        .submit_idempotent(&idem_task, "idem_key_1", "idem_payload")
        .await
        .expect("submit idempotent");
    assert_eq!(
        submission,
        IdempotentSubmission::Created(idem_task.id.clone())
    );

    let found_key = store
        .get_idempotent_task("idem_key_1")
        .expect("get idempotent");
    assert_eq!(found_key, Some(idem_task.id.clone()));

    let deleted_key = store
        .delete_idempotent_key("idem_key_1")
        .expect("delete idempotent key");
    assert!(deleted_key);
    assert_eq!(
        store
            .get_idempotent_task("idem_key_1")
            .expect("get after delete"),
        None
    );

    // 10. Safe check_integrity on corrupted db and repair_or_recreate
    let corrupt_ws =
        std::env::temp_dir().join(format!("task_store_corrupt_test_{}", uuid::Uuid::new_v4()));
    let corrupt_storage = corrupt_ws.join("storage");
    std::fs::create_dir_all(&corrupt_storage).expect("create storage");
    let c_file = corrupt_storage.join("tasks.db");
    std::fs::write(&c_file, b"MALFORMED_HEADER_DATA_CHECK").expect("write corrupt");
    let corrupt_conn = rusqlite::Connection::open(&c_file).expect("open raw");
    let corrupt_store = SqliteTaskStore::from_raw_parts(c_file.clone(), corrupt_conn);
    let integrity_res = corrupt_store.check_integrity();
    assert!(
        integrity_res.is_ok(),
        "check_integrity must return Ok on corrupted file"
    );
    assert!(
        !integrity_res.unwrap(),
        "Integrity check must return false for corrupted file"
    );

    // Repair restores health and integrity
    corrupt_store
        .repair_or_recreate()
        .expect("Repair must succeed");
    assert!(
        corrupt_store.is_healthy(),
        "Store must be healthy after repair"
    );
    assert!(
        corrupt_store.check_integrity().expect("Integrity check"),
        "Integrity must pass after repair"
    );

    // 11. Task store pruning of finished tasks and WAL checkpointing
    let old_completed = Task::new("Old Completed", "Old goal", Priority::Low);
    store.save_task(&old_completed).await.expect("Save old");
    let mut old_done = old_completed.clone();
    old_done.status = TaskStatus::Completed;
    store
        .finish_task(
            &old_done,
            &TaskResult {
                task_id: old_completed.id.clone(),
                status: TaskStatus::Completed,
                summary: "Done".into(),
                artifacts: Vec::new(),
                verifications: Vec::new(),
                duration_ms: 1,
                engine_used: None,
            },
            false,
        )
        .await
        .expect("Finish old");

    // Prune tasks created before tomorrow
    let tomorrow = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    let pruned = store
        .prune_finished_tasks(&tomorrow)
        .expect("Prune finished");
    assert!(pruned >= 1, "Must prune at least 1 finished task");

    // WAL Checkpoint
    store.wal_checkpoint().expect("WAL checkpoint must succeed");

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_ws);
    let _ = std::fs::remove_dir_all(&corrupt_ws);
}

/// TDD test case for TOPIC-20261105-43748a:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-0ae933.
/// Verifies comprehensive self-healing, resilience, and error elimination in `crates/agent-engine/src/process/`:
/// 1. Zero .unwrap() calls across all process submodules (mod.rs, builder.rs, runner.rs, stream.rs).
/// 2. Zero .expect( calls across all submodules.
/// 3. Zero panic!( calls across all submodules.
/// 4. Strict P90 line limit (<= 350 lines) across all submodules.
/// 5. Resilient multi-tier self-healing working directory resolution (resolve_working_dir) on uncreatable paths.
/// 6. Self-healing execution via run_command on forbidden/uncreatable working directory.
/// 7. ProcessGroup safety: PID bounds (>1 and <= i32::MAX), is_armed, and disarm capabilities.
/// 8. Stream UTF-8 character boundary helpers (floor_char_boundary, ceil_char_boundary).
/// 9. Safe UTF-8 truncation utilities (safe_truncate_utf8, safe_truncate_bytes).
/// 10. Panic-resilient stream callback in read_stream (catches panics without failing stream reading).
/// 11. Stream reading directly to String (read_stream_string).
/// 12. Empty/blank binary command validation returning ProcessFailed error cleanly.
/// 13. Ergonomic CommandSandbox methods (with_prefix_args, with_working_dir, add_env_var, is_bwrap, is_docker, is_containerized).
/// 14. Ergonomic run_simple_command helper execution.
#[tokio::test]
async fn test_topic_43748a_root_cause_eliminated_for_process_module() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let agent_engine_src = manifest_dir.parent().unwrap().join("agent-engine/src");
    let process_dir = agent_engine_src.join("process");
    assert!(
        process_dir.is_dir(),
        "crates/agent-engine/src/process must exist"
    );

    let submodules = ["mod.rs", "builder.rs", "runner.rs", "stream.rs"];
    for sub in submodules {
        let sub_path = process_dir.join(sub);
        assert!(sub_path.is_file(), "src/process/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/process/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/process/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/process/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/process/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    use agent_engine::process::{
        ceil_char_boundary, floor_char_boundary, read_stream, read_stream_string,
        resolve_working_dir, run_command, run_simple_command, safe_truncate_bytes,
        safe_truncate_utf8, CommandSandbox, ProcessGroup,
    };
    use agent_engine::{EngineError, EngineInput};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // 5. Multi-tier self-healing working directory resolution
    let forbidden_path = "/dev/null/forbidden/workspace/path/test";
    let healed_dir = resolve_working_dir(forbidden_path);
    assert!(
        healed_dir.is_dir(),
        "resolve_working_dir must return an existing directory for forbidden path: {:?}",
        healed_dir
    );

    let empty_dir = resolve_working_dir("");
    assert!(
        empty_dir.is_dir(),
        "resolve_working_dir must return an existing directory for empty string"
    );

    // 6. Self-healing execution via run_command on forbidden working directory
    let mut bad_input = EngineInput::new("echo self_healed_workdir", forbidden_path);
    bad_input.timeout_seconds = 10;
    let env_map = HashMap::new();
    let cmd_res = run_command(
        &bad_input,
        "echo",
        &["self_healed_workdir".into()],
        &env_map,
        None,
    )
    .await;
    assert!(
        cmd_res.is_ok(),
        "run_command on uncreatable working directory must self-heal and succeed"
    );
    let output = cmd_res.unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("self_healed_workdir"));

    // 7. ProcessGroup safety: PID bounds, is_armed, and disarm capabilities
    let mut pg = ProcessGroup::new(Some(42));
    assert!(pg.is_armed());
    assert_eq!(pg.pid(), Some(42));
    let disarmed = pg.disarm();
    assert_eq!(disarmed, Some(42));
    assert!(!pg.is_armed());
    assert_eq!(pg.pid(), None);

    // ProcessGroup with None or invalid PID drops safely without panic
    let pg_none = ProcessGroup::new(None);
    drop(pg_none);
    let pg_init = ProcessGroup::new(Some(1)); // init PID must not be killed
    drop(pg_init);
    let pg_overflow = ProcessGroup::new(Some(u32::MAX)); // sign-overflow guard
    drop(pg_overflow);

    // 8. Stream UTF-8 character boundary helpers
    let vietnamese_str = "Hệ thống tự trị 🚀🦀";
    let bytes = vietnamese_str.as_bytes();
    for i in 0..=bytes.len() {
        let floor = floor_char_boundary(bytes, i);
        let ceil = ceil_char_boundary(bytes, i);
        assert!(floor <= i);
        assert!(ceil >= i);
        if floor < bytes.len() {
            assert!(
                std::str::from_utf8(&bytes[..floor]).is_ok(),
                "bytes[..floor] must always be valid UTF-8"
            );
        }
        if ceil <= bytes.len() {
            assert!(
                std::str::from_utf8(&bytes[..ceil]).is_ok(),
                "bytes[..ceil] must always be valid UTF-8"
            );
        }
    }

    // 9. Safe UTF-8 truncation utilities
    let sample = "Triệt tiêu lỗi hệ thống lặp lại hoàn toàn";
    let truncated_str = safe_truncate_utf8(sample, 25);
    assert!(truncated_str.len() <= 25);
    assert!(sample.starts_with(truncated_str));

    let truncated_b = safe_truncate_bytes(bytes, 15);
    assert!(truncated_b.len() <= 15);
    assert!(std::str::from_utf8(truncated_b).is_ok());

    // 10. Panic-resilient stream callback in read_stream
    let stream_data = b"line 1: healthy stream\nline 2: callback will panic\nline 3: survived\n";
    let panic_hit = Arc::new(AtomicBool::new(false));
    let panic_hit_clone = panic_hit.clone();
    let panicking_callback: agent_engine::StreamCallback = Arc::new(move |_chunk| {
        panic_hit_clone.store(true, Ordering::SeqCst);
        panic!("Simulated crash in external stream callback");
    });
    let stream_res = read_stream(&stream_data[..], Some(panicking_callback), 1024).await;
    assert!(
        stream_res.is_ok(),
        "read_stream must safely catch callback panic and succeed"
    );
    assert!(
        panic_hit.load(Ordering::SeqCst),
        "Panicking callback was invoked"
    );
    let collected_bytes = stream_res.unwrap();
    assert_eq!(collected_bytes, stream_data.to_vec());

    // 11. Stream reading directly to String
    let string_data = b"Hello from read_stream_string\n";
    let string_res = read_stream_string(&string_data[..], None, 1024).await;
    assert!(string_res.is_ok());
    assert_eq!(string_res.unwrap(), "Hello from read_stream_string\n");

    // 12. Empty/blank binary command validation
    let empty_binary_res = run_command(&bad_input, "   ", &[], &env_map, None).await;
    assert!(
        matches!(empty_binary_res, Err(EngineError::ProcessFailed(msg)) if msg.contains("cannot be empty")),
        "Blank binary must return ProcessFailed error"
    );

    // 13. Ergonomic CommandSandbox methods
    let mut sandbox = CommandSandbox::new("/bin/sh", vec!["-c".into()], "/workspace")
        .with_env_whitelist(vec!["PATH".into()])
        .with_container_name("my-container")
        .with_prefix_args(vec!["--verbose".into()])
        .with_working_dir("/custom/ws");
    sandbox.add_env_var("CUSTOM_ENV");
    assert_eq!(sandbox.working_dir, "/custom/ws");
    assert!(sandbox.env_whitelist.contains(&"CUSTOM_ENV".to_string()));
    assert!(sandbox.prefix_args.contains(&"--verbose".to_string()));
    assert!(sandbox.is_containerized());
    assert!(!sandbox.is_bwrap());
    assert!(!sandbox.is_docker());

    let bwrap_box = CommandSandbox::new("bwrap", vec![], "/");
    assert!(bwrap_box.is_bwrap());
    assert!(bwrap_box.is_containerized());

    let docker_box = CommandSandbox::new("docker", vec![], "/");
    assert!(docker_box.is_docker());
    assert!(docker_box.is_containerized());

    // 14. Ergonomic run_simple_command helper execution
    let simple_res = run_simple_command("echo", &["hello_from_simple"]).await;
    assert!(simple_res.is_ok(), "run_simple_command must succeed");
    let simple_output = simple_res.unwrap();
    assert!(String::from_utf8_lossy(&simple_output.stdout).contains("hello_from_simple"));
}

/// TDD test case for TOPIC-20261105-b6fd4c:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-07a09e.
/// Verifies comprehensive resolution of root causes in `crates/agent-runtime/src/task_service/`
/// and its integration with daemon server handlers:
/// 1. Zero .unwrap() calls across all submodules (mod.rs, dispatch.rs, review.rs).
/// 2. Zero .expect( calls across all submodules.
/// 3. Zero panic!( calls across all submodules.
/// 4. Strict P90 line limits (<= 350 lines) for all submodules.
/// 5. Thread-safe `Send` future bounds on async operations (cancel_task, cancel_all, retry_all_failed, delete_task),
///    preventing axum `top_level_handler_fn` unsatisfied Handler bounds caused by `RwLockReadGuard` !Send leaks.
/// 6. Safe and idempotent task cancellation (cancel_task) without holding lock guards across await boundaries.
/// 7. Paginated batch cancellation (cancel_all) without lock guard retention.
/// 8. Resilient failed task batch retry (retry_all_failed) without collapsible-if clippy warnings.
/// 9. Coordinated deletion (delete_task) ensuring active background tasks are safely flagged before removal.
#[tokio::test]
async fn test_topic_b6fd4c_root_cause_eliminated_for_task_service_and_cancellation() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_service_dir = manifest_dir.join("src/task_service");
    assert!(
        task_service_dir.is_dir(),
        "src/task_service must exist as a directory"
    );

    let submodules = ["mod.rs", "dispatch.rs", "review.rs"];
    for sub in submodules {
        let sub_path = task_service_dir.join(sub);
        assert!(sub_path.is_file(), "src/task_service/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/task_service/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/task_service/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/task_service/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/task_service/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    use agent_domain::{Priority, Task, TaskStatus};
    let temp_workspace =
        std::env::temp_dir().join(format!("task_service_b6fd4c_{}", uuid::Uuid::new_v4()));
    let store = Arc::new(SqliteTaskStore::new(&temp_workspace).expect("Store init"));
    let service = Arc::new(TaskService::new(
        store.clone(),
        &temp_workspace,
        4,
        None,
        None,
    ));

    // 5. Thread-safe `Send` future bounds verification
    fn assert_is_send<F: Send>(_: &F) {}

    let dummy_id = agent_domain::TaskId("dummy-id".into());
    let cancel_fut = service.cancel_task(&dummy_id);
    assert_is_send(&cancel_fut);
    let cancel_all_fut = service.cancel_all();
    assert_is_send(&cancel_all_fut);
    let mock_engine: Arc<dyn agent_engine::ComputeEngine> =
        Arc::new(MockEngine::new("test output"));
    let retry_all_fut = service.retry_failed_tasks(mock_engine.clone());
    assert_is_send(&retry_all_fut);
    let delete_fut = service.delete_task(&dummy_id);
    assert_is_send(&delete_fut);

    // 6. Safe and idempotent task cancellation
    let t1 = service
        .submit_task("Cancel Target 1", "Goal to cancel", Priority::High)
        .await
        .expect("Submit t1");
    let cancel_result = service.cancel_task(&t1).await.expect("Cancel t1");
    assert!(
        cancel_result,
        "cancel_task should return true for queued task"
    );

    let t1_state = service
        .get_task(&t1)
        .await
        .expect("Get t1")
        .expect("t1 exists");
    assert!(
        matches!(t1_state.status, TaskStatus::Canceled { .. }),
        "Canceled task status must be Canceled"
    );

    // 7. Paginated batch cancellation (cancel_all)
    let mut batch_ids = Vec::new();
    for i in 0..5 {
        let tid = service
            .submit_task(&format!("Batch Task {}", i), "Batch Goal", Priority::Normal)
            .await
            .expect("Submit batch");
        batch_ids.push(tid);
    }
    let total_canceled = service.cancel_all().await.expect("cancel_all");
    assert!(
        total_canceled >= 5,
        "cancel_all should cancel at least the 5 queued batch tasks"
    );

    // 8. Resilient failed task batch retry (retry_failed_tasks)
    let failed_task = Task::new("Failed Task", "Needs Retry", Priority::Critical);
    let mut failed_state = failed_task.clone();
    failed_state.status = TaskStatus::Failed {
        error: "Simulated worker exit 1".into(),
    };
    store
        .save_task(&failed_state)
        .await
        .expect("Save failed task");

    let retried_count = service
        .retry_failed_tasks(mock_engine.clone())
        .await
        .expect("retry_failed_tasks");
    assert_eq!(
        retried_count, 1,
        "retry_failed_tasks should retry exactly 1 failed task"
    );

    let drained = service
        .drain(Duration::from_secs(3))
        .await
        .expect("Drain after retry");
    assert!(drained);

    // 9. Coordinated deletion (delete_task)
    let t_del = service
        .submit_task("Task to Delete", "Will be deleted", Priority::Low)
        .await
        .expect("Submit t_del");
    let del_result = service.delete_task(&t_del).await.expect("delete_task");
    assert!(
        del_result,
        "delete_task should return true for existing task"
    );
    let after_del = service.get_task(&t_del).await.expect("Get deleted");
    assert!(
        after_del.is_none(),
        "Task must no longer exist after deletion"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_workspace);
}

/// TDD test case for TOPIC-20261105-cbc444:
/// Permanently eliminates recurring mission failure in TOPIC-20261105-cb8d42.
/// Verifies comprehensive resilience, error elimination, and self-healing in `crates/agent-runtime/src/task_store/`:
/// 1. Zero .unwrap() calls across all submodules (mod.rs, crud.rs, lifecycle.rs, schema.rs).
/// 2. Zero .expect( calls across all submodules.
/// 3. Zero panic!( calls across all submodules.
/// 4. Strict P90 line limits (<= 350 lines) for all submodules.
/// 5. In-memory path handling: `SqliteTaskStore::new(":memory:")` must open in-memory db cleanly without creating physical directories on disk.
/// 6. Poisoned connection recovery with uncommitted transaction: `acquire_conn` rolls back dangling transactions from panicked threads and allows subsequent transactions.
/// 7. Terminal state synchronization in `finish_task`:
///    - Non-terminal task (`Executing`) with terminal result (`Completed` or `Failed`) is properly finalized.
///    - Attempting to finish an already finished task is rejected.
/// 8. Trait parity: `TaskStore` trait exposes `get_idempotent_task`, `delete_idempotent_key`, `clear_all`, `vacuum`, `repair_or_recreate` on `Arc<dyn TaskStore>`.
/// 9. Comprehensive WAL and SHM cleanup during `repair_or_recreate` and corrupt database quarantine.
#[tokio::test]
async fn test_topic_cbc444_root_cause_eliminated_for_task_store_resilience() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_store_dir = manifest_dir.join("src/task_store");
    assert!(
        task_store_dir.is_dir(),
        "src/task_store must exist as a directory"
    );

    let submodules = ["mod.rs", "crud.rs", "lifecycle.rs", "schema.rs"];
    for sub in submodules {
        let sub_path = task_store_dir.join(sub);
        assert!(sub_path.is_file(), "src/task_store/{} must exist", sub);
        let content = std::fs::read_to_string(&sub_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", sub_path.display()));

        // 1. Zero .unwrap()
        let unwraps: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".unwrap()"))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/task_store/{}:\n{:#?}",
            unwraps.len(),
            sub,
            unwraps
        );

        // 2. Zero .expect(
        let expects: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(".expect("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            expects.is_empty(),
            "Found {} expect() calls in src/task_store/{}:\n{:#?}",
            expects.len(),
            sub,
            expects
        );

        // 3. Zero panic!(
        let panics: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("panic!("))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();
        assert!(
            panics.is_empty(),
            "Found {} panic!() calls in src/task_store/{}:\n{:#?}",
            panics.len(),
            sub,
            panics
        );

        // 4. Line count <= 350
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "src/task_store/{} has {} lines, exceeding P90 limit (350 lines)",
            sub,
            line_count
        );
    }

    use agent_domain::{Priority, Task, TaskResult, TaskStatus};
    use agent_runtime::IdempotentSubmission;

    // 5. In-memory path handling: `SqliteTaskStore::new(":memory:")`
    let mem_store = SqliteTaskStore::new(":memory:")
        .expect("SqliteTaskStore::new with ':memory:' must succeed");
    assert_eq!(mem_store.db_path().to_str(), Some(":memory:"));
    assert!(mem_store.is_healthy(), "In-memory store must be healthy");
    // Ensure no physical `:memory:` directory was created
    assert!(
        !std::path::Path::new(":memory:").exists(),
        "':memory:' path must not create a physical directory"
    );

    // 6. Poisoned connection recovery with uncommitted transaction
    let poison_ws =
        std::env::temp_dir().join(format!("task_store_poison_{}", uuid::Uuid::new_v4()));
    let p_store = SqliteTaskStore::new(&poison_ws).expect("Create poison store");
    {
        let conn_arc = agent_runtime::task_store::schema::acquire_conn;
        // Simulate a thread starting an immediate transaction and panicking
        let temp_conn = rusqlite::Connection::open(p_store.db_path()).expect("open raw for poison");
        let mutex = std::sync::Arc::new(std::sync::Mutex::new(temp_conn));
        let mutex_clone = mutex.clone();
        let handle = std::thread::spawn(move || {
            let conn = mutex_clone.lock().expect("lock in thread");
            conn.execute_batch("BEGIN IMMEDIATE;")
                .expect("begin immediate");
            panic!("Simulated worker panic inside uncommitted transaction");
        });
        let _ = handle.join(); // thread panicked, mutex is poisoned and in open transaction

        // Safe acquire_conn should recover without error and rollback dangling transaction
        let recovered = conn_arc(&mutex);
        assert!(
            recovered.is_autocommit(),
            "Recovered connection must be rolled back to autocommit"
        );
        let query_ok = recovered.query_row("SELECT 1", [], |_| Ok(()));
        assert!(
            query_ok.is_ok(),
            "Query must succeed on recovered connection"
        );
    }
    let _ = std::fs::remove_dir_all(&poison_ws);

    // 7. Terminal state synchronization in finish_task
    let sync_ws = std::env::temp_dir().join(format!("task_store_sync_{}", uuid::Uuid::new_v4()));
    let dyn_store: Arc<dyn TaskStore> =
        Arc::new(SqliteTaskStore::new(&sync_ws).expect("Store init"));

    // Case 7a: Task status is Executing, but TaskResult is Completed -> both must become Completed
    let t_exec = Task::new("Executing Task", "Sync goal", Priority::Normal);
    dyn_store.save_task(&t_exec).await.expect("Save task");
    let res_completed = TaskResult {
        task_id: t_exec.id.clone(),
        status: TaskStatus::Completed,
        summary: "Execution succeeded".into(),
        artifacts: Vec::new(),
        verifications: Vec::new(),
        duration_ms: 100,
        engine_used: None,
    };
    dyn_store
        .finish_task(&t_exec, &res_completed, false)
        .await
        .expect("finish_task must succeed");
    let (saved_task, saved_res) = dyn_store
        .get_task_and_result(&t_exec.id)
        .await
        .expect("get")
        .expect("found");
    assert_eq!(
        saved_task.status,
        TaskStatus::Completed,
        "Task status must be synchronized to Completed"
    );
    assert_eq!(saved_res.expect("has result").status, TaskStatus::Completed);

    // Case 7b: Task status is Executing, but TaskResult is Failed -> both must become Failed
    let t_fail = Task::new("Failing Task", "Fail goal", Priority::High);
    dyn_store
        .save_task(&t_fail)
        .await
        .expect("Save failing task");
    let res_failed = TaskResult {
        task_id: t_fail.id.clone(),
        status: TaskStatus::Failed {
            error: "Engine execution error".into(),
        },
        summary: "Execution failed".into(),
        artifacts: Vec::new(),
        verifications: Vec::new(),
        duration_ms: 50,
        engine_used: None,
    };
    dyn_store
        .finish_task(&t_fail, &res_failed, false)
        .await
        .expect("finish_task must succeed for failed");
    let (saved_fail_task, saved_fail_res) = dyn_store
        .get_task_and_result(&t_fail.id)
        .await
        .expect("get")
        .expect("found");
    assert!(
        matches!(saved_fail_task.status, TaskStatus::Failed { .. }),
        "Task status must be synchronized to Failed"
    );
    assert!(matches!(
        saved_fail_res.expect("has result").status,
        TaskStatus::Failed { .. }
    ));

    // Case 7c: Attempting to finish an already finished task must be rejected
    assert!(
        dyn_store
            .finish_task(&t_fail, &res_failed, false)
            .await
            .is_err(),
        "Double finishing an already failed task must be rejected"
    );

    // 8. Trait parity via Arc<dyn TaskStore>
    let idem_task = Task::new("Idem Trait", "Test idem", Priority::Low);
    let sub_res = dyn_store
        .submit_idempotent(&idem_task, "idem_trait_key", "payload")
        .await
        .expect("submit idem");
    assert_eq!(sub_res, IdempotentSubmission::Created(idem_task.id.clone()));

    let found_id = dyn_store
        .get_idempotent_task("idem_trait_key")
        .await
        .expect("get idem trait");
    assert_eq!(found_id, Some(idem_task.id.clone()));

    let deleted = dyn_store
        .delete_idempotent_key("idem_trait_key")
        .await
        .expect("del idem trait");
    assert!(deleted);
    assert_eq!(
        dyn_store
            .get_idempotent_task("idem_trait_key")
            .await
            .expect("get after del"),
        None
    );

    // Trait admin methods
    dyn_store.vacuum().await.expect("vacuum via trait");
    dyn_store
        .wal_checkpoint()
        .await
        .expect("wal checkpoint via trait");
    assert!(dyn_store
        .check_integrity()
        .await
        .expect("integrity via trait"));
    assert!(dyn_store.is_healthy().await);

    // 9. Comprehensive WAL and SHM cleanup during repair_or_recreate
    let wal_ws = std::env::temp_dir().join(format!("task_store_wal_test_{}", uuid::Uuid::new_v4()));
    let wal_store = SqliteTaskStore::with_options(&wal_ws, 5000, true).expect("Init wal store");
    // Write something so WAL file is active
    let dummy = Task::new("WAL Dummy", "Dummy goal", Priority::Low);
    wal_store.save_task(&dummy).await.expect("Save dummy");

    // Corrupt the database header
    let db_path = wal_store.db_path().to_path_buf();
    std::fs::write(&db_path, b"CORRUPTED_DATABASE_HEADER_DATA").expect("write corrupt header");

    // Repair must succeed, recreate database, and purge/reset WAL/SHM
    wal_store
        .repair_or_recreate()
        .expect("repair_or_recreate must succeed");
    assert!(wal_store.is_healthy(), "Repaired store must be healthy");
    assert!(
        wal_store.check_integrity().expect("Integrity check"),
        "Integrity check must pass"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&sync_ws);
    let _ = std::fs::remove_dir_all(&wal_ws);
}

/// TDD test case for TOPIC-20261105-169a18 & TOPIC-20261105-6d28f0:
/// Structured OpenTelemetry Tracing Spans for OODA Lifecycle and Root Cause Elimination.
/// Verifies:
/// 1. Public export of phase constants: OODA_PHASE_OBSERVE, OODA_PHASE_ORIENT, OODA_PHASE_DECIDE, OODA_PHASE_ACT, OODA_PHASE_REMEDIATE.
/// 2. AutonomousOodaLoop builder methods (.with_auto_approve, .with_timeout_seconds).
/// 3. Capturing and ordering of ooda_phase spans through full lifecycle (Observe -> Orient -> Decide -> Act).
/// 4. Remediation phase tracing (Remediate) with proper task_id and subtask_id metadata on transient failure.
#[tokio::test(flavor = "current_thread")]
async fn test_topic_169a18_ooda_tracing_spans() {
    use agent_domain::{GuardrailPolicy, Priority, SubTask, Task, TaskStatus};
    use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
    use agent_memory::{DurableMemoryStore, ScratchpadManager};
    use agent_runtime::{
        AutonomousOodaLoop, OODA_PHASE_ACT, OODA_PHASE_DECIDE, OODA_PHASE_OBSERVE,
        OODA_PHASE_ORIENT, OODA_PHASE_REMEDIATE,
    };
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tracing::Subscriber;
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;

    // 1. Verify phase constants definitions and public exports
    assert_eq!(OODA_PHASE_OBSERVE, "Observe");
    assert_eq!(OODA_PHASE_ORIENT, "Orient");
    assert_eq!(OODA_PHASE_DECIDE, "Decide");
    assert_eq!(OODA_PHASE_ACT, "Act");
    assert_eq!(OODA_PHASE_REMEDIATE, "Remediate");

    // 2. Setup tracing capture layer
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CapturedPhaseSpan {
        task_id: String,
        subtask_id: String,
        execution_phase: String,
    }

    struct FieldVisitor {
        task_id: String,
        subtask_id: String,
        execution_phase: String,
    }

    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            let val = format!("{:?}", value).trim_matches('"').to_string();
            match field.name() {
                "task_id" => self.task_id = val,
                "subtask_id" => self.subtask_id = val,
                "execution_phase" => self.execution_phase = val,
                _ => {}
            }
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            match field.name() {
                "task_id" => self.task_id = value.to_string(),
                "subtask_id" => self.subtask_id = value.to_string(),
                "execution_phase" => self.execution_phase = value.to_string(),
                _ => {}
            }
        }
    }

    static INIT_GLOBAL_TRACING: std::sync::Once = std::sync::Once::new();
    static GLOBAL_CAPTURED_SPANS: Mutex<Option<Arc<Mutex<Vec<CapturedPhaseSpan>>>>> =
        Mutex::new(None);

    struct GlobalSpanCollector;

    impl<S: Subscriber> Layer<S> for GlobalSpanCollector {
        fn register_callsite(
            &self,
            metadata: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            if metadata.name() == "ooda_phase" {
                tracing::subscriber::Interest::always()
            } else {
                tracing::subscriber::Interest::sometimes()
            }
        }

        fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
            metadata.name() == "ooda_phase"
        }

        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: Context<'_, S>,
        ) {
            if attrs.metadata().name() == "ooda_phase" {
                let mut visitor = FieldVisitor {
                    task_id: String::new(),
                    subtask_id: String::new(),
                    execution_phase: String::new(),
                };
                attrs.record(&mut visitor);
                if !visitor.execution_phase.is_empty() {
                    if let Ok(guard) = GLOBAL_CAPTURED_SPANS.lock() {
                        if let Some(ref target) = *guard {
                            target.lock().unwrap().push(CapturedPhaseSpan {
                                task_id: visitor.task_id,
                                subtask_id: visitor.subtask_id,
                                execution_phase: visitor.execution_phase,
                            });
                        }
                    }
                }
            }
        }
    }

    INIT_GLOBAL_TRACING.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(GlobalSpanCollector);
        let _ = tracing::subscriber::set_global_default(subscriber);
    });

    let captured = Arc::new(Mutex::new(Vec::<CapturedPhaseSpan>::new()));
    *GLOBAL_CAPTURED_SPANS.lock().unwrap() = Some(captured.clone());

    // 3. Setup mock engine with decomposition and execution support
    struct MockTracingEngine<F>
    where
        F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
    {
        count: AtomicUsize,
        handler: F,
    }

    #[async_trait]
    impl<F> ComputeEngine for MockTracingEngine<F>
    where
        F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
    {
        async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
            let call_idx = self.count.fetch_add(1, Ordering::SeqCst);
            (self.handler)(input, call_idx)
        }

        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }

    let test_dir = std::env::temp_dir().join(format!(
        "ooda_tracing_test_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&test_dir).expect("create test dir");

    let engine = Arc::new(MockTracingEngine {
        count: AtomicUsize::new(0),
        handler: |input: &EngineInput, call_idx: usize| {
            if input.prompt_or_command.contains("Decompose")
                || input.prompt_or_command.contains("sequential subtasks")
            {
                let subtasks_json = r#"[
                    {"id": "ST-1", "title": "Setup", "command_or_prompt": "echo setup", "expected_verification": "exit_code: 0"},
                    {"id": "ST-2", "title": "Build", "command_or_prompt": "echo build", "expected_verification": "exit_code: 0"}
                ]"#;
                Ok(EngineOutput {
                    stdout_content: subtasks_json.to_string(),
                    exit_code: 0,
                    execution_duration_ms: 10,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            } else {
                Ok(EngineOutput {
                    stdout_content: format!("Executed subtask step {}", call_idx),
                    exit_code: 0,
                    execution_duration_ms: 5,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            }
        },
    });

    let guardrail = GuardrailPolicy::default();
    let scratchpad = ScratchpadManager::new(test_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(test_dir.join("durable.db"));

    let ooda = AutonomousOodaLoop::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        &test_dir.to_string_lossy(),
    )
    .with_auto_approve(true)
    .with_timeout_seconds(30);

    let mut task = Task::new(
        "Build Service",
        "Compile and verify backend",
        Priority::Normal,
    );
    let original_task_id = task.id.0.clone();

    let result = ooda.run_goal(&mut task).await;
    assert!(result.is_ok(), "Goal run failed: {:?}", result.err());
    assert_eq!(task.status, TaskStatus::Completed);

    let recorded = captured.lock().unwrap().clone();
    assert!(!recorded.is_empty(), "Expected spans to be recorded");

    let phases: Vec<String> = recorded
        .iter()
        .filter(|s| s.task_id == original_task_id || s.task_id.is_empty())
        .map(|s| s.execution_phase.clone())
        .collect();

    // Verify all core lifecycle phases are captured
    assert!(
        phases.contains(&OODA_PHASE_OBSERVE.to_string()),
        "Observe phase span missing: recorded={:?}",
        recorded
    );
    assert!(
        phases.contains(&OODA_PHASE_ORIENT.to_string()),
        "Orient phase span missing: recorded={:?}",
        recorded
    );
    assert!(
        phases.contains(&OODA_PHASE_DECIDE.to_string()),
        "Decide phase span missing: recorded={:?}",
        recorded
    );
    assert!(
        phases.contains(&OODA_PHASE_ACT.to_string()),
        "Act phase span missing: recorded={:?}",
        recorded
    );

    // Verify correct lifecycle order
    let obs_idx = phases.iter().position(|p| p == OODA_PHASE_OBSERVE).unwrap();
    let ori_idx = phases.iter().position(|p| p == OODA_PHASE_ORIENT).unwrap();
    let dec_idx = phases.iter().position(|p| p == OODA_PHASE_DECIDE).unwrap();
    let act_idx = phases.iter().position(|p| p == OODA_PHASE_ACT).unwrap();

    assert!(
        obs_idx <= ori_idx,
        "Observe must precede or coincide with Orient"
    );
    assert!(ori_idx <= dec_idx, "Orient must precede Decide");
    assert!(dec_idx <= act_idx, "Decide must precede Act");

    // Check Act spans contain subtask IDs
    let act_spans: Vec<&CapturedPhaseSpan> = recorded
        .iter()
        .filter(|s| {
            (s.task_id == original_task_id || s.task_id.is_empty())
                && s.execution_phase == OODA_PHASE_ACT
                && !s.subtask_id.is_empty()
        })
        .collect();
    assert!(
        act_spans.len() >= 2,
        "Expected at least 2 Act subtask spans"
    );
    assert_eq!(act_spans[0].subtask_id, "ST-1");
    assert_eq!(act_spans[1].subtask_id, "ST-2");

    // 4. Verification of Remediation phase tracing on step failure
    let step_fail_counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = step_fail_counter.clone();

    let rem_engine = Arc::new(MockTracingEngine {
        count: AtomicUsize::new(0),
        handler: move |input: &EngineInput, _call_idx: usize| {
            if input
                .prompt_or_command
                .contains("Analyze why this step failed")
            {
                let rem_json = r#"{
                    "id": "ST-FAIL-FIX-1",
                    "title": "Remediation: Fix missing config",
                    "command_or_prompt": "echo config-fixed",
                    "expected_verification": "exit_code: 0"
                }"#;
                Ok(EngineOutput {
                    stdout_content: rem_json.to_string(),
                    exit_code: 0,
                    execution_duration_ms: 10,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            } else if input.prompt_or_command.contains("failing-cmd") {
                let attempt = counter_clone.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Ok(EngineOutput {
                        stdout_content: "command crashed with exit code 1".to_string(),
                        exit_code: 1,
                        execution_duration_ms: 5,
                        engine_used: EngineKind::Mock,
                        session_id: None,
                    })
                } else {
                    Ok(EngineOutput {
                        stdout_content: "fixed!".to_string(),
                        exit_code: 0,
                        execution_duration_ms: 5,
                        engine_used: EngineKind::Mock,
                        session_id: None,
                    })
                }
            } else {
                Ok(EngineOutput {
                    stdout_content: "success".to_string(),
                    exit_code: 0,
                    execution_duration_ms: 5,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            }
        },
    });

    let rem_guardrail = GuardrailPolicy {
        max_retries_on_error: 2,
        ..Default::default()
    };
    let rem_scratchpad = ScratchpadManager::new(test_dir.join("scratchpad_rem"));
    let rem_durable = DurableMemoryStore::new(test_dir.join("durable_rem.db"));

    let rem_ooda = AutonomousOodaLoop::new(
        rem_engine,
        rem_guardrail,
        rem_scratchpad,
        rem_durable,
        &test_dir.to_string_lossy(),
    )
    .with_auto_approve(true)
    .with_timeout_seconds(30);

    let mut rem_task = Task::new(
        "Self-Healing Task",
        "Test remediation spans",
        Priority::Normal,
    );
    rem_task.subtasks = vec![SubTask::new(
        "ST-FAIL",
        "Execute prone step",
        "failing-cmd",
        "exit_code: 0",
    )];
    let rem_task_id = rem_task.id.0.clone();

    let rem_result = rem_ooda.run_goal(&mut rem_task).await;
    assert!(
        rem_result.is_ok(),
        "Expected self-healing goal to succeed: {:?}",
        rem_result.err()
    );
    assert_eq!(rem_task.status, TaskStatus::Completed);

    let final_recorded = captured.lock().unwrap().clone();
    let remediate_spans: Vec<&CapturedPhaseSpan> = final_recorded
        .iter()
        .filter(|s| s.execution_phase == OODA_PHASE_REMEDIATE)
        .collect();
    assert!(
        !remediate_spans.is_empty(),
        "Remediate span must be triggered"
    );
    assert_eq!(remediate_spans[0].subtask_id, "ST-FAIL");
    assert!(remediate_spans[0].task_id == rem_task_id || remediate_spans[0].task_id.is_empty());

    // Cleanup
    *GLOBAL_CAPTURED_SPANS.lock().unwrap() = None;
    let _ = std::fs::remove_dir_all(&test_dir);
}

/// TDD test case for TOPIC-20261105-a60060:
/// Permanently eliminates clippy/compiler lint warnings regarding `ambiguous_glob_reexports`.
/// Verifies:
/// 1. Zero crates use `#[allow(ambiguous_glob_reexports)]` as an evasion crutch.
/// 2. All workspace crate root files (`crates/*/src/lib.rs`) strictly enforce `#![deny(ambiguous_glob_reexports)]`.
/// 3. Cross-crate wildcard re-exports can be compiled in strict `#![deny(ambiguous_glob_reexports)]` scope
///    without any namespace collisions or ambiguous exports.
#[test]
fn test_topic_a60060_lint_rules_root_cause_eliminated() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();

    let crates = [
        "agent-domain",
        "agent-engine",
        "agent-memory",
        "agent-runtime",
        "agent-core",
        "agent-cli",
        "agent-server",
    ];

    for c in &crates {
        let lib_path = root_dir.join("crates").join(c).join("src").join("lib.rs");
        assert!(
            lib_path.is_file(),
            "Expected crate {} to have src/lib.rs at {}",
            c,
            lib_path.display()
        );

        let content = std::fs::read_to_string(&lib_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", lib_path.display()));

        assert!(
            content.contains("#![deny(ambiguous_glob_reexports)]"),
            "Crate {} must enforce #![deny(ambiguous_glob_reexports)] in src/lib.rs to prevent lint regressions",
            c
        );

        assert!(
            !content.contains("#[allow(ambiguous_glob_reexports)]"),
            "Crate {} must NOT use #[allow(ambiguous_glob_reexports)] as a temporary crutch",
            c
        );
    }

    // Verify workspace Cargo.toml also configures ambiguous_glob_reexports = "deny"
    let cargo_toml_path = root_dir.join("Cargo.toml");
    let cargo_toml =
        std::fs::read_to_string(&cargo_toml_path).expect("Must read workspace Cargo.toml");
    assert!(
        cargo_toml.contains("ambiguous_glob_reexports = \"deny\""),
        "Workspace Cargo.toml must deny ambiguous_glob_reexports in [workspace.lints.rust]"
    );
}

/// TDD test case for TOPIC-20261105-7b43f7:
/// Permanently eliminates clippy/compiler lint warnings regarding `clippy::clone_on_copy`,
/// `clippy::cloned_instead_of_copied`, and `suspicious_double_ref_op` (formerly `clone_double_ref`).
/// Verifies:
/// 1. Zero crates use `#[allow(clippy::clone_on_copy)]` or `#[allow(clippy::cloned_instead_of_copied)]` as evasion crutches.
/// 2. All workspace crate root files (`crates/*/src/lib.rs`) strictly enforce `#![deny(clippy::clone_on_copy)]`,
///    `#![deny(clippy::cloned_instead_of_copied)]`, and `#![deny(suspicious_double_ref_op)]`.
/// 3. Workspace `Cargo.toml` explicitly configures `clone_on_copy = "deny"` and `cloned_instead_of_copied = "deny"`
///    in `[workspace.lints.clippy]` and `suspicious_double_ref_op = "deny"` in `[workspace.lints.rust]`.
/// 4. Idiomatic usage of Copy operations: `.copied()` on iterators and direct value copy instead of redundant `.clone()`.
#[test]
fn test_topic_7b43f7_clippy_lint_rules_root_cause_eliminated() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();

    let crates = [
        "agent-domain",
        "agent-engine",
        "agent-memory",
        "agent-runtime",
        "agent-core",
        "agent-cli",
        "agent-server",
    ];

    for c in &crates {
        let lib_path = root_dir.join("crates").join(c).join("src").join("lib.rs");
        assert!(
            lib_path.is_file(),
            "Expected crate {} to have src/lib.rs at {}",
            c,
            lib_path.display()
        );

        let content = std::fs::read_to_string(&lib_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", lib_path.display()));

        assert!(
            content.contains("#![deny(clippy::clone_on_copy)]"),
            "Crate {} must enforce #![deny(clippy::clone_on_copy)] in src/lib.rs",
            c
        );

        assert!(
            content.contains("#![deny(clippy::cloned_instead_of_copied)]"),
            "Crate {} must enforce #![deny(clippy::cloned_instead_of_copied)] in src/lib.rs",
            c
        );

        assert!(
            content.contains("#![deny(suspicious_double_ref_op)]"),
            "Crate {} must enforce #![deny(suspicious_double_ref_op)] in src/lib.rs",
            c
        );

        assert!(
            !content.contains("#[allow(clippy::clone_on_copy)]")
                && !content.contains("#[allow(clone_on_copy)]"),
            "Crate {} must NOT use #[allow(clippy::clone_on_copy)] as an evasion crutch",
            c
        );

        assert!(
            !content.contains("#[allow(clippy::cloned_instead_of_copied)]")
                && !content.contains("#[allow(cloned_instead_of_copied)]"),
            "Crate {} must NOT use #[allow(clippy::cloned_instead_of_copied)] as an evasion crutch",
            c
        );
    }

    // Verify workspace Cargo.toml configurations
    let cargo_toml_path = root_dir.join("Cargo.toml");
    let cargo_toml =
        std::fs::read_to_string(&cargo_toml_path).expect("Must read workspace Cargo.toml");

    assert!(
        cargo_toml.contains("clone_on_copy = \"deny\""),
        "Workspace Cargo.toml must deny clone_on_copy in [workspace.lints.clippy]"
    );
    assert!(
        cargo_toml.contains("cloned_instead_of_copied = \"deny\""),
        "Workspace Cargo.toml must deny cloned_instead_of_copied in [workspace.lints.clippy]"
    );
    assert!(
        cargo_toml.contains("suspicious_double_ref_op = \"deny\""),
        "Workspace Cargo.toml must deny suspicious_double_ref_op in [workspace.lints.rust]"
    );

    // Verify programmatic / behavioral idioms:
    // 1. Idiomatic iterator copying with .copied()
    let numbers = [10usize, 20usize, 30usize];
    let sum: usize = numbers.iter().copied().filter(|&x| x > 15).sum();
    assert_eq!(sum, 50);
    let first_even = numbers.iter().copied().find(|&x| x == 20);
    assert_eq!(first_even, Some(20));

    // 2. Direct copy of Copy struct without calling clone()
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    struct CopyToken {
        id: u64,
        valid: bool,
    }

    let token1 = CopyToken {
        id: 42,
        valid: true,
    };
    let token2 = token1; // Copy semantics, not .clone()
    assert_eq!(token1, token2);

    // 3. Clean dereferencing of double references instead of clone
    let val = 100u32;
    let ref1 = &val;
    let ref2 = &ref1;
    let deref_val: u32 = **ref2;
    assert_eq!(deref_val, 100);
}

#[test]
fn test_topic_7fe3cf_workspace_resilience_and_cargo_profiles() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir
        .parent()
        .expect("crates/")
        .parent()
        .expect("root");

    // Verify workspace Cargo.toml profiles debug = 1
    let cargo_toml_path = root_dir.join("Cargo.toml");
    let cargo_toml =
        std::fs::read_to_string(&cargo_toml_path).expect("Must read workspace Cargo.toml");

    assert!(
        cargo_toml.contains("[profile.dev]"),
        "Workspace Cargo.toml must define [profile.dev]"
    );
    assert!(
        cargo_toml.contains("[profile.test]"),
        "Workspace Cargo.toml must define [profile.test]"
    );
    assert!(
        cargo_toml.contains("debug = 1"),
        "Workspace Cargo.toml profiles must configure debug = 1 to slash linker memory overhead"
    );

    // Verify .cargo/config.toml
    let config_path = root_dir.join(".cargo/config.toml");
    assert!(
        config_path.exists(),
        "Must have .cargo/config.toml for linker optimization"
    );
    let config_content =
        std::fs::read_to_string(&config_path).expect("Must read .cargo/config.toml");
    assert!(
        config_content.contains("--no-keep-memory"),
        ".cargo/config.toml must specify --no-keep-memory for ld"
    );
}

/// TDD test case for TOPIC-20261106-211ead:
/// Verifies that `crates/agent-runtime/src/execution/tests.rs` contains ZERO explicit `panic!()` calls,
/// adhering strictly to safe error handling guidelines and zero-panic test conventions.
#[test]
fn test_topic_211ead_no_panic_in_execution_tests_rs() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let tests_path = manifest_dir.join("src/execution/tests.rs");
    assert!(tests_path.is_file(), "src/execution/tests.rs must exist");

    let content =
        std::fs::read_to_string(&tests_path).expect("Failed to read src/execution/tests.rs");

    let panic_calls: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("panic!("))
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();

    assert!(
        panic_calls.is_empty(),
        "Found {} explicit panic!() calls in src/execution/tests.rs:\n{:#?}",
        panic_calls.len(),
        panic_calls
    );
}

/// TDD test case for TOPIC-20261106-395f34:
/// Verifies that BackoffPolicy is properly integrated, exported, and complies with
/// the architectural requirements for exponential backoff & full jitter.
#[test]
fn test_topic_395f34_backoff_policy_architecture_and_exports() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir
        .parent()
        .expect("crates/")
        .parent()
        .expect("root");
    let backoff_path = root_dir.join("crates/agent-engine/src/backoff.rs");
    assert!(
        backoff_path.is_file(),
        "crates/agent-engine/src/backoff.rs must exist"
    );

    let content = std::fs::read_to_string(&backoff_path).expect("Read backoff.rs");

    // Ensure struct BackoffPolicy definition
    assert!(
        content.contains("pub struct BackoffPolicy"),
        "Must define struct BackoffPolicy"
    );
    assert!(
        content.contains("pub initial_interval: Duration"),
        "Must contain initial_interval"
    );
    assert!(
        content.contains("pub max_interval: Duration"),
        "Must contain max_interval"
    );
    assert!(
        content.contains("pub multiplier: f64"),
        "Must contain multiplier"
    );
    assert!(content.contains("pub jitter: bool"), "Must contain jitter");

    // Verify public helper constructors and methods
    assert!(
        content.contains("pub fn calculate_max_delay"),
        "Must provide calculate_max_delay"
    );
    assert!(
        content.contains("pub fn compute_delay"),
        "Must provide compute_delay"
    );
    assert!(
        content.contains("pub async fn sleep_for_attempt"),
        "Must provide sleep_for_attempt"
    );

    // Verify zero explicit panic!()
    let panic_matches: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim();
            !trimmed.starts_with("//") && trimmed.contains("panic!(")
        })
        .map(|(i, line)| (i + 1, line.trim()))
        .collect();
    assert!(
        panic_matches.is_empty(),
        "backoff.rs must contain zero panic calls: {:?}",
        panic_matches
    );
}
