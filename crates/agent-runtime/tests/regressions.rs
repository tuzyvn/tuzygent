use agent_domain::{GuardrailPolicy, Priority, SubTask, Task, TaskStatus};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{
    AutonomousOodaLoop, SandboxConfig, SandboxDriverKind, WorkflowDefinition, WorkflowOrchestrator,
};
use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("tuzygent-regression-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct RetryEngine {
    executions: AtomicUsize,
    always_fail: bool,
    transport_failure: bool,
}
#[async_trait]
impl ComputeEngine for RetryEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let (exit_code, text) = if input
            .prompt_or_command
            .contains("Autonomous AI Problem Solver")
        {
            (
                0,
                r#"{"id":"fix","title":"fix","command_or_prompt":"fix","expected_verification":"fixed"}"#,
            )
        } else if input.prompt_or_command == "original" {
            let count = self.executions.fetch_add(1, Ordering::SeqCst);
            if count == 0 || self.always_fail {
                if self.transport_failure {
                    return Err(EngineError::ProcessFailed("probe failure".into()));
                }
                (1, "failed")
            } else {
                (0, "ok")
            }
        } else {
            (0, "fixed")
        };
        Ok(EngineOutput {
            stdout_content: text.into(),
            exit_code,
            execution_duration_ms: 0,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}
#[tokio::test]
async fn remediation_retries_original_for_exit_and_transport_failures() {
    for transport_failure in [false, true] {
        for always_fail in [false, true] {
            let workspace = Workspace::new();
            let engine = Arc::new(RetryEngine {
                executions: AtomicUsize::new(0),
                always_fail,
                transport_failure,
            });
            let sandbox = SandboxConfig {
                driver: SandboxDriverKind::Host,
                ..Default::default()
            };
            let ooda = AutonomousOodaLoop::new(
                engine.clone(),
                GuardrailPolicy::default_enterprise(),
                ScratchpadManager::new(&workspace.0),
                DurableMemoryStore::new(&workspace.0),
                workspace.path(),
            )
            .with_sandbox_config(sandbox);
            let mut task = Task::new("retry", "retry", Priority::High);
            task.subtasks
                .push(SubTask::new("original", "original", "original", "done"));
            let result = ooda.run_goal(&mut task).await.unwrap();
            let original = task.subtasks.iter().find(|s| s.id == "original").unwrap();
            if always_fail {
                assert!(matches!(result, TaskStatus::Failed { .. }));
                assert!(matches!(original.status, TaskStatus::Failed { .. }));
                assert_eq!(engine.executions.load(Ordering::SeqCst), 4);
            } else {
                assert_eq!(result, TaskStatus::Completed);
                assert!(task
                    .subtasks
                    .iter()
                    .all(|s| s.status == TaskStatus::Completed));
                assert_eq!(engine.executions.load(Ordering::SeqCst), 2);
            }
        }
    }
}
fn workflow(nodes: serde_json::Value, edges: serde_json::Value) -> WorkflowDefinition {
    serde_json::from_value(
        serde_json::json!({"id":"review","name":"review","nodes":nodes,"edges":edges}),
    )
    .unwrap()
}
#[tokio::test]
async fn failed_node_and_approval_block_dependents() {
    let workspace = Workspace::new();
    let executor = WorkflowOrchestrator::new(workspace.path(), workspace.path());
    for (kind, data, expected, node_status) in [
        (
            "toolNode",
            serde_json::json!({"command":"exit 7","enableSelfHealing":false}),
            "Failed",
            "failed",
        ),
        (
            "approvalNode",
            serde_json::json!({"requiredRole":"admin"}),
            "WaitingApproval",
            "waiting_approval",
        ),
    ] {
        let marker = workspace.0.join("must-not-run");
        let workflow = workflow(
            serde_json::json!([
                {"id":"a","type":kind,"data":data},
                {"id":"b","type":"toolNode","data":{"command":format!("touch {}", marker.display())}}
            ]),
            serde_json::json!([{"id":"edge","source":"a","target":"b"}]),
        );
        let report = executor
            .execute_workflow(
                &workflow,
                Some(serde_json::json!({"approved":true,"role":"admin"})),
                "test",
            )
            .await
            .unwrap();
        assert_eq!(report.status, expected);
        assert_eq!(report.node_results["a"].status, node_status);
        assert_eq!(report.node_results["b"].status, "skipped");
        assert!(!marker.exists());
    }
}
#[tokio::test]
async fn invalid_graphs_are_rejected_before_side_effects() {
    let workspace = Workspace::new();
    let executor = WorkflowOrchestrator::new(workspace.path(), workspace.path());
    let marker = workspace.0.join("must-not-run");
    let nodes = serde_json::json!([
        {"id":"root","type":"toolNode","data":{"command":format!("touch {}", marker.display())}},
        {"id":"a","type":"triggerNode","data":{}},{"id":"b","type":"triggerNode","data":{}}
    ]);
    for edges in [
        serde_json::json!([{"id":"ab","source":"a","target":"b"},{"id":"ba","source":"b","target":"a"}]),
        serde_json::json!([{"id":"bad","source":"missing","target":"a"}]),
    ] {
        assert!(executor
            .execute_workflow(&workflow(nodes.clone(), edges), None, "test")
            .await
            .is_err());
        assert!(!marker.exists());
    }
    assert!(executor
        .execute_workflow(
            &workflow(serde_json::json!([]), serde_json::json!([])),
            None,
            "test"
        )
        .await
        .is_err());
}

#[tokio::test]
async fn sandbox_context_reaches_planning_actions_and_remediation() {
    struct Sandboxed {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl ComputeEngine for Sandboxed {
        async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
            let sandbox = input
                .command_sandbox
                .as_ref()
                .expect("missing sandbox context");
            assert_eq!(sandbox.program, "docker");
            assert_eq!(sandbox.working_dir, "/workspace");
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let text = if call == 0 {
                assert!(input.prompt_or_command.contains("Planner"));
                r#"[{"id":"a","title":"a","command_or_prompt":"action","expected_verification":"done"}]"#
            } else if call == 2 {
                assert!(input.prompt_or_command.contains("Problem Solver"));
                r#"{"id":"fix","title":"fix","command_or_prompt":"fix","expected_verification":"fixed"}"#
            } else {
                "ok"
            };
            Ok(EngineOutput {
                stdout_content: text.into(),
                exit_code: if call == 1 { 1 } else { 0 },
                execution_duration_ms: 0,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }
    let workspace = Workspace::new();
    let engine = Arc::new(Sandboxed {
        calls: AtomicUsize::new(0),
    });
    let config = SandboxConfig {
        driver: SandboxDriverKind::Docker,
        ..Default::default()
    };
    let ooda = AutonomousOodaLoop::new(
        engine.clone(),
        GuardrailPolicy::default_enterprise(),
        ScratchpadManager::new(&workspace.0),
        DurableMemoryStore::new(&workspace.0),
        workspace.path(),
    )
    .with_sandbox_config(config);
    let mut task = Task::new("goal", "goal", Priority::High);
    assert_eq!(
        ooda.run_goal(&mut task).await.unwrap(),
        TaskStatus::Completed
    );
    assert_eq!(engine.calls.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn rig_propagates_engine_failures_and_actually_runs_channel_handoffs() {
    use agent_runtime::KernelOperator;
    struct Capture {
        calls: AtomicUsize,
        fail: bool,
    }
    #[async_trait]
    impl ComputeEngine for Capture {
        async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if input.prompt_or_command.contains("Tiếp nhận") {
                assert!(
                    input.system_context.contains("source-output"),
                    "source output missing from handoff"
                );
            }
            Ok(EngineOutput {
                stdout_content: "source-output".into(),
                exit_code: if self.fail { 7 } else { 0 },
                execution_duration_ms: 0,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }
    let workspace = Workspace::new();
    let spec: agent_domain::RigSpec = serde_json::from_value(serde_json::json!({
        "rig":{"id":"test","name":"test"},"operator":{"agent_id":"lead"},
        "seats":[
            {"seat_id":"dev","role":"dev","capabilities":["code"],"harness":"process","engine":"probe"},
            {"seat_id":"qa","role":"qa","capabilities":["review"],"harness":"process","engine":"probe"}
        ],"channels":[{"from_seat":"dev","to_seat":"qa","mode":"review"}]
    })).unwrap();
    for fail in [false, true] {
        let engine = Arc::new(Capture {
            calls: AtomicUsize::new(0),
            fail,
        });
        let operator = KernelOperator::new(spec.clone(), workspace.path())
            .with_engine("probe", engine.clone());
        let report = operator.execute_goal("code", None).await.unwrap();
        assert_eq!(report.status, if fail { "Failed" } else { "Completed" });
        assert_eq!(
            engine.calls.load(Ordering::SeqCst),
            if fail { 1 } else { 2 }
        );
        if fail {
            assert_eq!(report.assignments[0].status, "Failed");
        } else {
            assert_eq!(report.channel_dispatches[0].status, "Completed");
        }
    }
}

#[cfg(unix)]
fn codex_workspace() -> Workspace {
    use std::os::unix::fs::PermissionsExt;
    let workspace = Workspace::new();
    let binary = workspace.0.join("codex");
    std::fs::write(
        &binary,
        include_str!("../../agent-engine/tests/fixtures/codex.sh"),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = format!(
        r#"
        [defaults]
        engine = "codex"
        sandbox_driver = "host"
        timeout_seconds = 5
        [security]
        allowed_env_vars = []
        mount_credentials = []
        [engines.codex]
        kind = "codex"
        binary_path = {:?}
        model = "configured-model"
        [engines.codex.env]
        CODEX_TEST_CASE = "ooda"
        CODEX_TEST_ARGS = {:?}
    "#,
        binary.to_str().unwrap(),
        workspace.0.join("argv").to_str().unwrap()
    );
    std::fs::write(workspace.0.join("tuzygent.toml"), config).unwrap();
    workspace
}

#[cfg(unix)]
#[tokio::test]
async fn codex_delegation_runs_through_ooda_and_publishes_protocol_events() {
    let workspace = codex_workspace();
    let config = agent_engine::EnginesConfig::load_hierarchical(Some(&workspace.0));
    let engine = agent_engine::configured_engine(&config, "codex", None).unwrap();
    let bus = agent_runtime::InterAgentBus::new(64);
    let mut receiver = bus.subscribe();
    let ooda = AutonomousOodaLoop::new(
        engine,
        GuardrailPolicy::default_enterprise(),
        ScratchpadManager::new(&workspace.0),
        DurableMemoryStore::new(&workspace.0),
        workspace.path(),
    )
    .with_sandbox_config(SandboxConfig::from_engines_config(&config).unwrap())
    .with_bus(bus);
    let mut task = Task::new("Delegate", "Do the task with Codex", Priority::High);
    assert_eq!(
        ooda.run_goal(&mut task).await.unwrap(),
        TaskStatus::Completed
    );
    assert_eq!(task.subtasks.len(), 1);
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    assert!(events
        .iter()
        .any(|event| event.event_type == "task.engine_event"
            && event.payload["event"]["type"] == "thread.started"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "task.stream_delta"
            && event.payload["delta"] == "CODEX_DELEGATED"));
}

#[cfg(unix)]
#[tokio::test]
async fn rig_and_workflow_select_codex_and_preserve_model_overrides() {
    let workspace = codex_workspace();
    let spec: agent_domain::RigSpec = serde_json::from_value(serde_json::json!({
        "rig":{"id":"codex","name":"codex"}, "operator":{"agent_id":"lead"},
        "seats":[{"seat_id":"dev","role":"developer","engine":"codex","model":"seat-model"}]
    }))
    .unwrap();
    let operator = agent_runtime::KernelOperator::new(spec, workspace.path());
    assert_eq!(
        operator
            .resolve_engine(Some("codex"), None)
            .unwrap()
            .engine_kind(),
        EngineKind::Codex
    );
    assert!(operator.resolve_engine(Some("typo"), None).is_err());
    let report = operator.execute_goal("delegate code", None).await.unwrap();
    assert_eq!(report.status, "Completed");
    assert_eq!(
        report.assignments[0].output.as_deref(),
        Some("CODEX_DELEGATED")
    );
    let args = std::fs::read_to_string(workspace.0.join("argv")).unwrap();
    assert!(args.contains("--model\nseat-model\n"));
    let graph = workflow(
        serde_json::json!([{"id":"code","type":"agentNode","data":{"engine":"codex","model":"workflow-model","promptTemplate":"delegate"}}]),
        serde_json::json!([]),
    );
    let report = WorkflowOrchestrator::new(workspace.path(), workspace.path())
        .execute_workflow(&graph, None, "test")
        .await
        .unwrap();
    assert_eq!(report.status, "Completed");
    assert_eq!(
        report.node_results["code"].output["stdout"],
        "CODEX_DELEGATED"
    );
    assert!(std::fs::read_to_string(workspace.0.join("argv"))
        .unwrap()
        .contains("--model\nworkflow-model\n"));
}
