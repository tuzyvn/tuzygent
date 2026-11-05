use agent_domain::{
    GuardrailPolicy, Priority, SubTask, Task, TaskStatus,
};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{
    AutonomousOodaLoop,
    OODA_PHASE_ACT, OODA_PHASE_DECIDE, OODA_PHASE_OBSERVE, OODA_PHASE_ORIENT, OODA_PHASE_REMEDIATE,
};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

// ============================================================================
// Tracing Span Capture Layer
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
struct CapturedPhaseSpan {
    task_id: String,
    subtask_id: String,
    execution_phase: String,
}

#[derive(Default)]
struct SpanCollector {
    spans: Arc<Mutex<Vec<CapturedPhaseSpan>>>,
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

impl<S: Subscriber> Layer<S> for SpanCollector {
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, _id: &tracing::span::Id, _ctx: Context<'_, S>) {
        if attrs.metadata().name() == "ooda_phase" {
            let mut visitor = FieldVisitor {
                task_id: String::new(),
                subtask_id: String::new(),
                execution_phase: String::new(),
            };
            attrs.record(&mut visitor);
            if !visitor.execution_phase.is_empty() {
                self.spans.lock().unwrap().push(CapturedPhaseSpan {
                    task_id: visitor.task_id,
                    subtask_id: visitor.subtask_id,
                    execution_phase: visitor.execution_phase,
                });
            }
        }
    }
}

// ============================================================================
// Mock Engine & Test Environment
// ============================================================================

struct MockTracingEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    count: AtomicUsize,
    handler: F,
}

impl<F> MockTracingEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    fn new(handler: F) -> Self {
        Self {
            count: AtomicUsize::new(0),
            handler,
        }
    }
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

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!("{}_{}", prefix, uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test(flavor = "current_thread")]
async fn test_ooda_lifecycle_spans_in_order() {
    let captured = Arc::new(Mutex::new(Vec::<CapturedPhaseSpan>::new()));
    let collector = SpanCollector {
        spans: captured.clone(),
    };
    let subscriber = tracing_subscriber::registry().with(collector);

    let _guard = tracing::subscriber::set_default(subscriber);

    let test_dir = TestDir::new("ooda_tracing_test");
    let test_path = test_dir.path.clone();

    let engine = Arc::new(MockTracingEngine::new(move |input, call_idx| {
        if input.prompt_or_command.contains("Decompose") || input.prompt_or_command.contains("sequential subtasks") {
            // Planner / Decide decomposition output
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
            // Subtask execution Act
            Ok(EngineOutput {
                stdout_content: format!("Executed subtask step {}", call_idx),
                exit_code: 0,
                execution_duration_ms: 5,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
    }));

    let guardrail = GuardrailPolicy::default();
    let scratchpad = ScratchpadManager::new(test_path.join("scratchpad"));
    let durable = DurableMemoryStore::new(test_path.join("durable.db"));

    let ooda = AutonomousOodaLoop::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        &test_path.to_string_lossy(),
    )
    .with_auto_approve(true)
    .with_timeout_seconds(30);

    let mut task = Task::new("Build Service", "Compile and verify backend", Priority::Normal);
    let original_task_id = task.id.0.clone();

    let result = ooda.run_goal(&mut task).await;
    assert!(result.is_ok(), "Goal run failed: {:?}", result.err());
    assert_eq!(task.status, TaskStatus::Completed);

    let recorded = captured.lock().unwrap().clone();
    assert!(!recorded.is_empty(), "Expected spans to be recorded");

    // Filter by our task_id
    let phases: Vec<String> = recorded
        .iter()
        .filter(|s| s.task_id == original_task_id || s.task_id.is_empty())
        .map(|s| s.execution_phase.clone())
        .collect();

    // Verify presence of all expected lifecycle phases
    assert!(phases.contains(&OODA_PHASE_OBSERVE.to_string()), "Observe phase span missing");
    assert!(phases.contains(&OODA_PHASE_ORIENT.to_string()), "Orient phase span missing");
    assert!(phases.contains(&OODA_PHASE_DECIDE.to_string()), "Decide phase span missing");
    assert!(phases.contains(&OODA_PHASE_ACT.to_string()), "Act phase span missing");

    // Verify ordering: Observe -> Orient -> Decide -> Act
    let obs_idx = phases.iter().position(|p| p == OODA_PHASE_OBSERVE).unwrap();
    let ori_idx = phases.iter().position(|p| p == OODA_PHASE_ORIENT).unwrap();
    let dec_idx = phases.iter().position(|p| p == OODA_PHASE_DECIDE).unwrap();
    let act_idx = phases.iter().position(|p| p == OODA_PHASE_ACT).unwrap();

    assert!(obs_idx <= ori_idx, "Observe must precede or coincide with Orient");
    assert!(ori_idx <= dec_idx, "Orient must precede Decide");
    assert!(dec_idx <= act_idx, "Decide must precede Act");

    // Check Act spans contain subtask IDs
    let act_spans: Vec<&CapturedPhaseSpan> = recorded
        .iter()
        .filter(|s| s.execution_phase == OODA_PHASE_ACT && !s.subtask_id.is_empty())
        .collect();
    assert!(act_spans.len() >= 2, "Expected at least 2 Act subtask spans");
    assert_eq!(act_spans[0].subtask_id, "ST-1");
    assert_eq!(act_spans[1].subtask_id, "ST-2");
}

#[tokio::test(flavor = "current_thread")]
async fn test_ooda_remediation_span_metadata_and_backtracking() {
    let captured = Arc::new(Mutex::new(Vec::<CapturedPhaseSpan>::new()));
    let collector = SpanCollector {
        spans: captured.clone(),
    };
    let subscriber = tracing_subscriber::registry().with(collector);

    let _guard = tracing::subscriber::set_default(subscriber);

    let test_dir = TestDir::new("ooda_remediation_tracing_test");
    let test_path = test_dir.path.clone();

    let step_fail_counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = step_fail_counter.clone();

    let engine = Arc::new(MockTracingEngine::new(move |input, _call_idx| {
        if input.prompt_or_command.contains("Analyze why this step failed") {
            // Remediation subtask formulation
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
                // First attempt fails
                Ok(EngineOutput {
                    stdout_content: "command crashed with exit code 1".to_string(),
                    exit_code: 1,
                    execution_duration_ms: 5,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            } else {
                // Retry passes
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
    }));

    let guardrail = GuardrailPolicy {
        max_retries_on_error: 2,
        ..Default::default()
    };

    let scratchpad = ScratchpadManager::new(test_path.join("scratchpad"));
    let durable = DurableMemoryStore::new(test_path.join("durable.db"));

    let ooda = AutonomousOodaLoop::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        &test_path.to_string_lossy(),
    )
    .with_auto_approve(true)
    .with_timeout_seconds(30);

    let mut task = Task::new("Self-Healing Task", "Test remediation spans", Priority::Normal);
    task.subtasks = vec![
        SubTask::new("ST-FAIL", "Execute prone step", "failing-cmd", "exit_code: 0"),
    ];
    let task_id = task.id.0.clone();

    let result = ooda.run_goal(&mut task).await;
    assert!(result.is_ok(), "Expected self-healing goal to succeed: {:?}", result.err());
    assert_eq!(task.status, TaskStatus::Completed);

    let recorded = captured.lock().unwrap().clone();

    // Check Remediate span exists with appropriate metadata
    let remediate_spans: Vec<&CapturedPhaseSpan> = recorded
        .iter()
        .filter(|s| s.execution_phase == OODA_PHASE_REMEDIATE)
        .collect();
    assert!(!remediate_spans.is_empty(), "Remediate span must be triggered");

    let rem_span = remediate_spans[0];
    assert_eq!(rem_span.subtask_id, "ST-FAIL");
    assert!(rem_span.task_id == task_id || rem_span.task_id.is_empty());

    // Verify chronological sequence: Act (failure) -> Remediate -> Act (fix) -> Act (original)
    let sequence: Vec<&str> = recorded
        .iter()
        .map(|s| s.execution_phase.as_str())
        .collect();

    let first_act = sequence.iter().position(|&p| p == OODA_PHASE_ACT).unwrap();
    let remediate_pos = sequence.iter().position(|&p| p == OODA_PHASE_REMEDIATE).unwrap();
    let last_act = sequence.iter().rposition(|&p| p == OODA_PHASE_ACT).unwrap();

    assert!(first_act < remediate_pos, "Act must precede Remediate");
    assert!(remediate_pos < last_act, "Remediate must be followed by Act");
}
