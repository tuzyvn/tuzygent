use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use agent_memory::{
    simulate_context_fault, ContextBuilder, ContextBuilderError, ContextBuilderFault,
    ContextBuilderFaultPolicy, ResilientContextBuilder,
};

/// TDD test case for TOPIC-20261105-83ce28 & TOPIC-20261105-d71323:
/// Chaos Engineering and Fault Injection resilience in agent-memory::context_builder.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-d71323.
#[test]
fn test_topic_83ce28_d71323_context_builder_chaos_resilience() {
    let builder = ContextBuilder::new("Implement distributed fault-tolerant ledger")
        .with_lessons(vec!["Always verify ledger cryptographic hash chain".to_string()])
        .with_recent_logs(vec!["Log: Phase 1 consensus verified".to_string()])
        .add_subtask_step("ST-01".to_string(), "Scaffolded module".to_string())
        .with_max_total_chars(8000);

    // 1. Intermittent socket timeout recovery
    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::SocketTimeout, 2);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);
    let res = resilient.build();
    assert!(res.is_ok(), "ResilientContextBuilder should recover from intermittent socket timeout: {:?}", res.err());
    let (sys, work) = res.unwrap();
    assert!(sys.contains("PRIMARY GOAL"));
    assert!(sys.contains("Implement distributed fault-tolerant ledger"));
    assert!(work.contains("ST-01"));
    assert_eq!(resilient.built_count(), 1);
    assert!(resilient.recovered_count() > 0);
}

#[test]
fn test_context_builder_intermittent_connection_reset_recovery() {
    let builder = ContextBuilder::new("Autonomous remediation goal")
        .with_recent_logs(vec!["Step A done".to_string()]);

    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::ConnectionReset, 2);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(res.is_ok(), "Must recover from intermittent connection reset");
    assert_eq!(resilient.built_count(), 1);
    assert!(resilient.recovered_count() > 0);
}

#[test]
fn test_context_builder_transient_io_recovery() {
    let builder = ContextBuilder::new("Process telemetry data")
        .with_lessons(vec!["Handle disk buffers cleanly".to_string()]);

    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::TransientIo, 2);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(res.is_ok(), "Must recover from transient IO failure");
    assert_eq!(resilient.built_count(), 1);
    assert!(resilient.recovered_count() > 0);
}

#[test]
fn test_context_builder_persistent_timeout_exceeds_retries() {
    let builder = ContextBuilder::new("Mission impossible");
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::SocketTimeout);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 2);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::SocketTimeout)));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);
}

#[test]
fn test_context_builder_persistent_connection_reset_exceeds_retries() {
    let builder = ContextBuilder::new("Persistent failure goal");
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::ConnectionReset);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 2);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::ConnectionReset)));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);
}

#[test]
fn test_context_builder_corrupted_context_rejection() {
    let builder = ContextBuilder::new("Goal with corrupt incoming fragment");
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::CorruptedContext);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::CorruptedContext(_))));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.rejected_count(), 1);
}

#[test]
fn test_context_builder_budget_exceeded_rejection() {
    let builder = ContextBuilder::new("Massive context goal").with_max_total_chars(1000);
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::BudgetExceeded);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(matches!(
        res,
        Err(ContextBuilderError::BudgetExceeded { required: _, max_budget: _ })
    ));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.rejected_count(), 1);
}

#[test]
fn test_context_builder_empty_goal_validation_failure() {
    let builder = ContextBuilder::new("   ");
    let policy = ContextBuilderFaultPolicy::none();
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::EmptyGoal)));
}

#[test]
fn test_context_builder_audit_ledger_recording() {
    let mut ledger = AuditLedger::new();
    let builder = ContextBuilder::new("Audit tracked goal")
        .with_recent_logs(vec!["Activity 1".to_string()]);

    let policy = ContextBuilderFaultPolicy::none();
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build_and_record(&mut ledger);
    assert!(res.is_ok());

    let entries = ledger.entries();
    assert!(!entries.is_empty());
    assert_eq!(entries[0].event_type, AuditEventType::CommandAllowed);
    assert_eq!(entries[0].severity, ActionSeverity::Routine);
    assert!(entries[0].details.contains("Audit tracked goal"));

    // Now test recording a blocked event on validation failure
    let invalid_builder = ContextBuilder::new("");
    let mut resilient_invalid = ResilientContextBuilder::new(invalid_builder, ContextBuilderFaultPolicy::none(), 3);
    let err_res = resilient_invalid.build_and_record(&mut ledger);
    assert!(matches!(err_res, Err(ContextBuilderError::EmptyGoal)));

    let updated_entries = ledger.entries();
    assert!(updated_entries.len() >= 2);
    let last = &updated_entries[updated_entries.len() - 1];
    assert_eq!(last.event_type, AuditEventType::CommandBlocked);
    assert_eq!(last.severity, ActionSeverity::Destructive);
}

#[test]
fn test_simulate_context_fault_direct() {
    assert_eq!(
        simulate_context_fault(&ContextBuilderFault::SocketTimeout),
        Err(ContextBuilderError::SocketTimeout)
    );
    assert_eq!(
        simulate_context_fault(&ContextBuilderFault::ConnectionReset),
        Err(ContextBuilderError::ConnectionReset)
    );
    assert!(matches!(
        simulate_context_fault(&ContextBuilderFault::TransientIo),
        Err(ContextBuilderError::TransientIo(_))
    ));
    assert!(matches!(
        simulate_context_fault(&ContextBuilderFault::CorruptedContext),
        Err(ContextBuilderError::CorruptedContext(_))
    ));
    assert!(matches!(
        simulate_context_fault(&ContextBuilderFault::BudgetExceeded),
        Err(ContextBuilderError::BudgetExceeded { .. })
    ));
}

#[test]
fn test_high_volume_context_builder_resilience() {
    let builder = ContextBuilder::new("High volume test goal")
        .with_lessons(vec!["Continuous resilience verification".to_string()]);

    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::SocketTimeout, 3)
        .with_max_injections(10);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 5);

    for _ in 0..50 {
        let res = resilient.build();
        assert!(res.is_ok());
    }

    assert_eq!(resilient.built_count(), 50);
    assert_eq!(resilient.recovered_count(), 10);
}
