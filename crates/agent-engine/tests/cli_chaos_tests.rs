use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_domain::{AuditEventType, AuditLedger};
use agent_engine::{
    simulate_cli_fault, CliError, CliFault, CliFaultPolicy, ComputeEngine, EngineError,
    EngineInput, EngineKind, EngineOutput, GenericCliEngine, ResilientCliEngine,
    ResilientCliWrapper,
};

fn create_test_input(prompt: &str, timeout_seconds: u64) -> EngineInput {
    EngineInput {
        on_event: None,
        command_sandbox: None,
        prompt_or_command: prompt.to_string(),
        system_context: "You are a test CLI agent".to_string(),
        working_dir: ".".to_string(),
        timeout_seconds,
        on_stream_chunk: None,
        temperature: None,
        max_tokens: None,
        max_output_bytes: None,
    }
}

struct MockFailingCliEngine {
    failures_remaining: Arc<AtomicUsize>,
    fail_error: EngineError,
    success_output: String,
}

#[async_trait::async_trait]
impl ComputeEngine for MockFailingCliEngine {
    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let current = self.failures_remaining.load(Ordering::SeqCst);
        if current > 0 {
            self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            Err(match &self.fail_error {
                EngineError::Timeout => EngineError::Timeout,
                EngineError::ProcessFailed(msg) => EngineError::ProcessFailed(msg.clone()),
                EngineError::CliNotFound(msg) => EngineError::CliNotFound(msg.clone()),
                EngineError::ApiRequestFailed(msg) => EngineError::ApiRequestFailed(msg.clone()),
                EngineError::PolicyRejected(msg) => EngineError::PolicyRejected(msg.clone()),
            })
        } else {
            Ok(EngineOutput {
                stdout_content: self.success_output.clone(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Generic("mock-failing-cli".to_string()),
                session_id: None,
            })
        }
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Generic("mock-failing-cli".to_string())
    }
}

#[test]
fn test_cli_fault_simulation_and_taxonomy() {
    assert_eq!(
        simulate_cli_fault(&CliFault::SocketTimeout),
        Err(CliError::SocketTimeout)
    );
    assert_eq!(
        simulate_cli_fault(&CliFault::ProcessTimeout),
        Err(CliError::ProcessTimeout)
    );
    assert_eq!(
        simulate_cli_fault(&CliFault::BrokenPipe),
        Err(CliError::BrokenPipe)
    );
    assert!(matches!(
        simulate_cli_fault(&CliFault::TransientIo),
        Err(CliError::TransientIo(_))
    ));
    assert!(matches!(
        simulate_cli_fault(&CliFault::ProcessFailed),
        Err(CliError::ProcessFailed(_))
    ));
    assert!(matches!(
        simulate_cli_fault(&CliFault::CorruptedOutput),
        Err(CliError::CorruptedOutput(_))
    ));
    assert!(matches!(
        simulate_cli_fault(&CliFault::CliNotFound),
        Err(CliError::CliNotFound(_))
    ));

    // Verify mapping to EngineError
    assert!(matches!(
        CliFault::SocketTimeout.to_engine_error(),
        EngineError::Timeout
    ));
    assert!(matches!(
        CliFault::ProcessTimeout.to_engine_error(),
        EngineError::Timeout
    ));
    assert!(matches!(
        CliFault::BrokenPipe.to_engine_error(),
        EngineError::ProcessFailed(_)
    ));
    assert!(matches!(
        CliFault::TransientIo.to_engine_error(),
        EngineError::ProcessFailed(_)
    ));
    assert!(matches!(
        CliFault::CorruptedOutput.to_engine_error(),
        EngineError::ProcessFailed(_)
    ));
    assert!(matches!(
        CliFault::CliNotFound.to_engine_error(),
        EngineError::CliNotFound(_)
    ));
}

#[test]
fn test_cli_fault_policy_intermittent_and_max_injections() {
    let mut policy =
        CliFaultPolicy::intermittent(CliFault::ProcessTimeout, 2).with_max_injections(2);

    assert_eq!(policy.should_inject(), Some(CliFault::ProcessTimeout)); // invocation 1
    assert_eq!(policy.should_inject(), None); // invocation 2
    assert_eq!(policy.should_inject(), Some(CliFault::ProcessTimeout)); // invocation 3
    assert_eq!(policy.should_inject(), None); // max injections (2) reached
    assert_eq!(policy.injected_count(), 2);
    assert_eq!(policy.invocation_count(), 3);

    policy.reset();
    assert_eq!(policy.invocation_count(), 0);
    assert_eq!(policy.injected_count(), 0);
}

#[tokio::test]
async fn test_generic_cli_baseline_success() {
    let engine = GenericCliEngine::new(
        "test_echo",
        "echo",
        vec!["{prompt}".to_string()],
        HashMap::new(),
        None,
    );
    let input = create_test_input("hello-chaos-cli", 5);
    let output = engine.execute(&input).await.expect("echo should succeed");

    assert_eq!(output.exit_code, 0);
    assert!(output.stdout_content.contains("hello-chaos-cli"));
}

#[tokio::test]
async fn test_resilient_cli_engine_recovers_intermittent_process_timeout() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(2)),
        fail_error: EngineError::Timeout,
        success_output: "recovered-after-timeout".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok(), "Expected recovery after transient timeouts");
    let output = res.unwrap();
    assert_eq!(output.stdout_content, "recovered-after-timeout");
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
    assert_eq!(resilient.rejected_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_engine_recovers_broken_pipe() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(1)),
        fail_error: EngineError::ProcessFailed("Broken pipe (os error 32)".to_string()),
        success_output: "recovered-after-broken-pipe".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap().stdout_content, "recovered-after-broken-pipe");
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_resilient_cli_engine_recovers_transient_io_failure() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(2)),
        fail_error: EngineError::ProcessFailed(
            "Transient I/O error: Resource temporarily unavailable".to_string(),
        ),
        success_output: "recovered-after-transient-io".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_resilient_cli_engine_chaos_injection_recovery() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(0)),
        fail_error: EngineError::Timeout,
        success_output: "injected-chaos-recovered".to_string(),
    };

    // Policy injects 2 transient faults: ProcessTimeout and BrokenPipe
    let policy = CliFaultPolicy::always(CliFault::ProcessTimeout).with_max_injections(2);
    let resilient = ResilientCliEngine::new(mock, policy, 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_resilient_cli_engine_fails_when_exceeding_max_retries() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(5)),
        fail_error: EngineError::Timeout,
        success_output: "should-not-reach".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 2);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_err());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.resolved_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_engine_corrupted_output_not_retried() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(0)),
        fail_error: EngineError::Timeout,
        success_output: "should-not-reach".to_string(),
    };

    // Corrupted output is non-transient, reject immediately
    let policy = CliFaultPolicy::always(CliFault::CorruptedOutput);
    let resilient = ResilientCliEngine::new(mock, policy, 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_err());
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.recovered_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_engine_cli_not_found_not_retried() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(0)),
        fail_error: EngineError::Timeout,
        success_output: "should-not-reach".to_string(),
    };

    // Missing binary should fail fast without retry
    let policy = CliFaultPolicy::always(CliFault::CliNotFound);
    let resilient = ResilientCliEngine::new(mock, policy, 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), EngineError::CliNotFound(_)));
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.recovered_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_audit_ledger_recorded_flow() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(1)),
        fail_error: EngineError::ProcessFailed("Broken pipe".to_string()),
        success_output: "ledger-verified-output".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let mut ledger = AuditLedger::new();
    let input = create_test_input("audit-test", 5);

    let res = resilient.execute_and_record(&mut ledger, &input).await;
    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);

    // Audit ledger should have recorded 1 retry attempt + 1 final success
    assert_eq!(ledger.len(), 2);
    let entries = ledger.entries();
    assert_eq!(entries[0].event_type, AuditEventType::CommandAllowed);
    assert!(entries[0].details.contains("retrying attempt 1/3"));
    assert_eq!(entries[1].event_type, AuditEventType::CommandAllowed);
    assert!(entries[1].details.contains("CLI execution succeeded"));
}

#[tokio::test]
async fn test_resilient_cli_with_generic_engine_live_echo() {
    let generic = GenericCliEngine::new(
        "live_echo",
        "echo",
        vec!["{prompt}".to_string()],
        HashMap::new(),
        None,
    );

    // Inject 1 fault via policy, resilient wrapper should retry and succeed
    let policy = CliFaultPolicy::always(CliFault::ProcessTimeout).with_max_injections(1);
    let resilient = generic.with_chaos(policy, 3);

    let input = create_test_input("live-echo-resilience-test", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    let output = res.unwrap();
    assert!(output.stdout_content.contains("live-echo-resilience-test"));
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
}
