use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use async_trait::async_trait;

use agent_domain::{ActionSeverity, AuditEventType, AuditLedger, GuardrailPolicy};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_runtime::{
    simulate_execution_fault, ExecutionFault, ExecutionFaultPolicy, ExecutionService,
    ResilientExecutionService,
};

/// Mock engine for chaos testing that records invocations and can simulate failures
struct ChaosMockEngine {
    invocations: Arc<AtomicUsize>,
    output_prefix: String,
    fail_first_n: usize,
    panic_first_n: usize,
}

impl ChaosMockEngine {
    fn new(output_prefix: impl Into<String>) -> Self {
        Self {
            invocations: Arc::new(AtomicUsize::new(0)),
            output_prefix: output_prefix.into(),
            fail_first_n: 0,
            panic_first_n: 0,
        }
    }

    fn failing_first(output_prefix: impl Into<String>, n: usize) -> Self {
        Self {
            invocations: Arc::new(AtomicUsize::new(0)),
            output_prefix: output_prefix.into(),
            fail_first_n: n,
            panic_first_n: 0,
        }
    }

    fn panicking_first(output_prefix: impl Into<String>, n: usize) -> Self {
        Self {
            invocations: Arc::new(AtomicUsize::new(0)),
            output_prefix: output_prefix.into(),
            fail_first_n: 0,
            panic_first_n: n,
        }
    }

    fn invocation_count(&self) -> usize {
        self.invocations.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ComputeEngine for ChaosMockEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let count = self.invocations.fetch_add(1, Ordering::SeqCst);
        if count < self.panic_first_n {
            panic!("Intentional chaos panic inside ComputeEngine::execute");
        }
        if count < self.fail_first_n {
            return Err(EngineError::ProcessFailed(format!(
                "Intentional engine process failure on invocation {}",
                count + 1
            )));
        }

        Ok(EngineOutput {
            stdout_content: format!("{}: {}", self.output_prefix, input.prompt_or_command),
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

#[test]
fn test_execution_fault_simulation_direct() {
    let faults = vec![
        ExecutionFault::SocketTimeout,
        ExecutionFault::ConnectionReset,
        ExecutionFault::TransientIo,
        ExecutionFault::ServiceUnavailable,
        ExecutionFault::WorkerCrash,
        ExecutionFault::CapacityOverload,
        ExecutionFault::CorruptedOutput,
        ExecutionFault::Panic,
    ];

    for fault in faults {
        let err = fault.to_engine_error();
        assert_eq!(format!("{:?}", err), format!("{:?}", simulate_execution_fault(&fault).unwrap_err()));
    }
}

#[tokio::test]
async fn test_intermittent_socket_timeout_recovery() {
    let engine = Arc::new(ChaosMockEngine::new("socket_recovery"));
    let service = ExecutionService::new("/tmp", None, None);
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::SocketTimeout, 2);
    let mut resilient = ResilientExecutionService::new(service, policy, 3);

    let input = EngineInput::new("echo 'chaos-socket-test'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok(), "Expected success after recovery: {:?}", result.err());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_intermittent_connection_reset_recovery() {
    let engine = Arc::new(ChaosMockEngine::new("conn_reset"));
    let service = ExecutionService::new("/tmp", None, None);
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::ConnectionReset, 2);
    let mut resilient = ResilientExecutionService::new(service, policy, 3);

    let input = EngineInput::new("echo 'chaos-conn-reset'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_transient_io_failure_recovery() {
    let engine = Arc::new(ChaosMockEngine::new("transient_io"));
    let service = ExecutionService::new("/tmp", None, None);
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::TransientIo, 2);
    let mut resilient = ResilientExecutionService::new(service, policy, 4);

    let input = EngineInput::new("echo 'chaos-io'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_service_unavailable_recovery() {
    let engine = Arc::new(ChaosMockEngine::new("service_503"));
    let service = ExecutionService::new("/tmp", None, None);
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::ServiceUnavailable, 2);
    let mut resilient = ResilientExecutionService::new(service, policy, 3);

    let input = EngineInput::new("echo 'chaos-503'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_worker_crash_recovery() {
    let engine = Arc::new(ChaosMockEngine::new("worker_crash"));
    let service = ExecutionService::new("/tmp", None, None);
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::WorkerCrash, 2);
    let mut resilient = ResilientExecutionService::new(service, policy, 3);

    let input = EngineInput::new("echo 'chaos-crash'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_persistent_failure_exceeds_max_retries() {
    let engine = Arc::new(ChaosMockEngine::new("persistent_fail"));
    let service = ExecutionService::new("/tmp", None, None);
    // Always inject SocketTimeout, max retries = 2
    let policy = ExecutionFaultPolicy::always(ExecutionFault::SocketTimeout);
    let mut resilient = ResilientExecutionService::new(service, policy, 2);

    let input = EngineInput::new("echo 'chaos-permanent'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_err());
    assert_eq!(resilient.failed_count(), 1);
    assert_eq!(resilient.total_executed(), 0);
}

#[tokio::test]
async fn test_panic_resilience_and_recovery() {
    // Inject panic fault intermittently with 3 retries
    let engine = Arc::new(ChaosMockEngine::new("panic_test"));
    let service = ExecutionService::new("/tmp", None, None);
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::Panic, 2);
    let mut resilient = ResilientExecutionService::new(service, policy, 3);

    let input = EngineInput::new("echo 'panic-resilience'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok(), "Expected recovery after caught panic");
    assert!(resilient.panics_caught_count() > 0);
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_engine_internal_panic_is_isolated() {
    // Engine that panics on the first invocation, then succeeds
    let engine = Arc::new(ChaosMockEngine::panicking_first("panicking_engine", 1));
    let service = ExecutionService::new("/tmp", None, None);
    let mut resilient = ResilientExecutionService::new(service, ExecutionFaultPolicy::none(), 2);

    let input = EngineInput::new("echo 'internal-panic'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok(), "Expected recovery after engine internal panic: {:?}", result.err());
    assert!(resilient.panics_caught_count() > 0);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_policy_violation_not_retried() {
    let engine = Arc::new(ChaosMockEngine::new("policy_violation"));
    let guardrail = GuardrailPolicy::default_enterprise();
    let service = ExecutionService::new("/tmp", None, Some(guardrail));
    let mut resilient = ResilientExecutionService::new(service, ExecutionFaultPolicy::none(), 5);

    // Dangerous command violating red-line
    let input = EngineInput::new("rm -rf /", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(matches!(result, Err(EngineError::PolicyRejected(_))));
    // Engine was never invoked
    assert_eq!(engine.invocation_count(), 0);
    // Should NOT have attempted multiple retries on policy violation
    assert_eq!(resilient.recovered_count(), 0);
    assert_eq!(resilient.failed_count(), 1);
}

#[tokio::test]
async fn test_audit_ledger_recording_success_and_failure() {
    let mut ledger = AuditLedger::new();
    let engine = Arc::new(ChaosMockEngine::new("audit_test"));
    let service = ExecutionService::new("/tmp", None, None);
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::ConnectionReset, 2);
    let mut resilient = ResilientExecutionService::new(service, policy, 3);

    let input = EngineInput::new("echo 'audit-test-entry'", "/tmp");
    let result = resilient.execute_and_record(&mut ledger, engine.clone(), &input).await;

    assert!(result.is_ok());
    let entries = ledger.entries();
    assert!(!entries.is_empty());
    let last = entries.last().unwrap();
    assert_eq!(last.event_type, AuditEventType::CommandAllowed);
    assert_eq!(last.severity, ActionSeverity::Routine);

    // Now test recording of policy rejection
    let bad_input = EngineInput::new("rm -rf /", "/tmp");
    let bad_res = resilient.execute_and_record(&mut ledger, engine.clone(), &bad_input).await;
    assert!(bad_res.is_err());
    let last_bad = ledger.entries().last().unwrap();
    assert_eq!(last_bad.event_type, AuditEventType::CommandBlocked);
    assert_eq!(last_bad.severity, ActionSeverity::Destructive);
}

#[tokio::test]
async fn test_output_limit_with_chaos_truncation() {
    let engine = Arc::new(ChaosMockEngine::new("long_output_generator_with_a_very_long_string_payload"));
    let mut guardrail = GuardrailPolicy::default_enterprise();
    guardrail.max_output_bytes = 30;

    let service = ExecutionService::new("/tmp", None, Some(guardrail));
    let mut resilient = ResilientExecutionService::new(service, ExecutionFaultPolicy::none(), 2);

    let input = EngineInput::new("generate long string output", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.stdout_content.contains("[... output truncated by ExecutionService guardrail ...]"));
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_engine_process_failure_recovery() {
    // Engine that fails with ProcessFailed on the first 2 attempts, then succeeds
    let engine = Arc::new(ChaosMockEngine::failing_first("failing_engine", 2));
    let service = ExecutionService::new("/tmp", None, None);
    let mut resilient = ResilientExecutionService::new(service, ExecutionFaultPolicy::none(), 3);

    let input = EngineInput::new("echo 'process-fail-recovery'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok(), "Expected recovery after engine process failures: {:?}", result.err());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.total_executed(), 1);
}

#[tokio::test]
async fn test_high_volume_concurrent_chaos_resilience() {
    let service = ExecutionService::new("/tmp", None, None);
    let engine = Arc::new(ChaosMockEngine::new("concurrent"));

    let mut handles = Vec::new();
    for i in 0..10 {
        let eng = engine.clone();
        let srv = service.clone();
        let handle = tokio::spawn(async move {
            let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::TransientIo, 3);
            let mut resilient = ResilientExecutionService::new(srv, policy, 3);
            let input = EngineInput::new(format!("task_{}", i), "/tmp");
            resilient.execute_resilient(eng, &input).await
        });
        handles.push(handle);
    }

    for h in handles {
        let res = h.await.expect("Join failed");
        assert!(res.is_ok(), "Concurrent execution task failed: {:?}", res.err());
    }
}

/// TDD test case for TOPIC-20261105-5a31cd & TOPIC-20261105-b635d0:
/// Chaos Engineering and Fault Injection resilience in agent-runtime::execution.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-b635d0.
#[tokio::test]
async fn test_topic_5a31cd_b635d0_execution_chaos_resilience() {
    let service = ExecutionService::new("/tmp", None, None);
    let engine = Arc::new(ChaosMockEngine::new("topic_5a31cd_b635d0"));
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::SocketTimeout, 2).with_max_injections(4);
    let mut resilient = ResilientExecutionService::new(service, policy, 3);

    let input = EngineInput::new("echo 'chaos-mission-resilience'", "/tmp");
    let result = resilient.execute_resilient(engine.clone(), &input).await;

    assert!(result.is_ok(), "Must survive intermittent fault injection without failing mission");
    assert_eq!(resilient.total_executed(), 1);
    assert!(resilient.recovered_count() > 0);
}

