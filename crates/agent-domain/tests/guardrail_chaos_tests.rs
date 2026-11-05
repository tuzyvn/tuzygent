use agent_domain::{
    ActionSeverity, AuditLedger, GuardrailError, GuardrailFault, GuardrailFaultPolicy,
    GuardrailPolicy, ResilientGuardrailEvaluator,
};

#[test]
fn test_guardrail_validation_and_rule_integrity() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::none();
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    // Normal safe command must pass
    assert!(evaluator.check_command_safety("cargo test --all").is_ok());
    assert_eq!(evaluator.evaluated_count(), 1);

    // Red line violation must be blocked
    let res = evaluator.check_command_safety("rm -rf /var/data");
    assert!(matches!(res, Err(GuardrailError::Blocked(_))));
    assert_eq!(evaluator.blocked_count(), 1);
}

#[test]
fn test_guardrail_intermittent_socket_timeout_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    // Intermittent socket timeout every 2 invocations, max retries 3
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::SocketTimeout, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("git status");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
    assert_eq!(evaluator.evaluated_count(), 1);
}

#[test]
fn test_guardrail_max_retries_exceeded_on_persistent_timeout() {
    let policy = GuardrailPolicy::default_enterprise();
    // Always inject SocketTimeout, max retries 2
    let fault_policy = GuardrailFaultPolicy::always(GuardrailFault::SocketTimeout);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 2);

    let res = evaluator.check_command_safety("cargo check");
    assert_eq!(res, Err(GuardrailError::SocketTimeout));
    assert_eq!(evaluator.evaluated_count(), 0);
}

#[test]
fn test_guardrail_connection_reset_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::ConnectionReset, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("echo 'safe command'");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
}

#[test]
fn test_guardrail_transient_io_failure_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::TransientIo, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("ls -la");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
}

#[test]
fn test_guardrail_corrupted_rule_rejection() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::always(GuardrailFault::CorruptedRule);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("cargo test");
    assert!(matches!(res, Err(GuardrailError::CorruptedPolicy(_))));
    assert_eq!(evaluator.evaluated_count(), 0);
}

#[test]
fn test_guardrail_service_unavailable_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::ServiceUnavailable, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("python3 script.py");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
}

#[test]
fn test_guardrail_audit_ledger_chaos_integration() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::SocketTimeout, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);
    let mut ledger = AuditLedger::new();

    // 1. Safe command under intermittent chaos
    let res1 = evaluator.evaluate_and_record(&mut ledger, "cargo build --release");
    assert!(res1.is_ok());

    // 2. Blocked command under intermittent chaos
    let res2 = evaluator.evaluate_and_record(&mut ledger, "DROP TABLE orders");
    assert!(matches!(res2, Err(GuardrailError::Blocked(_))));

    // 3. Verify audit ledger chain integrity
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 2);
}

#[test]
fn test_guardrail_audit_ledger_persistent_failure_records_exception() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::always(GuardrailFault::SocketTimeout);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 2);
    let mut ledger = AuditLedger::new();

    let res = evaluator.evaluate_and_record(&mut ledger, "cargo run");
    assert_eq!(res, Err(GuardrailError::SocketTimeout));

    // Ledger should record the exception event
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 1);
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::HighImpact);
}

#[test]
fn test_high_volume_guardrail_chaos_resilience() {
    let policy = GuardrailPolicy::default_enterprise();
    // Intermittent connection reset every 3 invocations with max retries 4
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::ConnectionReset, 3);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 4);
    let mut ledger = AuditLedger::new();

    for i in 0..50 {
        let cmd = if i % 5 == 0 {
            "rm -rf /tmp/target"
        } else {
            "cargo test"
        };
        let _ = evaluator.evaluate_and_record(&mut ledger, cmd);
    }

    assert_eq!(ledger.entries().len(), 50);
    assert!(ledger.verify_integrity().is_ok());
    assert!(evaluator.recovered_count() > 0);
    assert_eq!(evaluator.blocked_count(), 10);
    assert_eq!(evaluator.evaluated_count(), 40);
}
