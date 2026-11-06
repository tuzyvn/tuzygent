use agent_domain::{
    ActionSeverity, AuditLedger, AgentId, EventError, EventFault, EventFaultPolicy,
    GuardrailError, GuardrailFault, GuardrailFaultPolicy, GuardrailPolicy, InterAgentEvent,
    Priority, ResilientEventProcessor, ResilientGuardrailEvaluator, SubTask, Task, TaskError,
    TaskStatus,
};

/// Reproduction test for TOPIC-20261105-d9881e / TOPIC-20261105-cae92b:
/// Chaos Engineering and Fault Injection resilience in agent-domain::event
#[test]
fn test_reproduce_and_verify_fault_injection_resilience() {
    println!("[Reproduction Test] Initializing Chaos Engineering & Fault Injection verification...");

    let agent_sender = AgentId::new("agent-tester");
    let agent_receiver = AgentId::new("agent-worker");

    let event = InterAgentEvent::new(
        agent_sender,
        Some(agent_receiver),
        "reproduction.event",
        serde_json::json!({ "scenario": "chaos_injection", "retry_threshold": 3 }),
    );

    // Verify baseline event integrity
    assert!(event.validate().is_ok(), "Base event must be structurally valid");

    // Scenario 1: Intermittent Socket Timeout Recovery
    println!("[Reproduction Test] Testing intermittent socket timeout recovery...");
    let timeout_policy = EventFaultPolicy::intermittent(EventFault::SocketTimeout, 2);
    let mut timeout_processor = ResilientEventProcessor::new(timeout_policy, 3);
    let timeout_res = timeout_processor.process_event(event.clone());
    assert!(timeout_res.is_ok(), "Processor must recover from intermittent socket timeout");
    assert_eq!(timeout_processor.processed_count(), 1);
    assert!(timeout_processor.recovered_count() > 0);

    // Scenario 2: Connection Reset Recovery
    println!("[Reproduction Test] Testing connection reset recovery...");
    let reset_policy = EventFaultPolicy::intermittent(EventFault::ConnectionReset, 2);
    let mut reset_processor = ResilientEventProcessor::new(reset_policy, 3);
    let reset_res = reset_processor.process_event(event.clone());
    assert!(reset_res.is_ok(), "Processor must recover from intermittent connection reset");
    assert_eq!(reset_processor.processed_count(), 1);
    assert!(reset_processor.recovered_count() > 0);

    // Scenario 3: Corrupted Payload Rejection
    println!("[Reproduction Test] Testing corrupted payload rejection...");
    let corrupted_policy = EventFaultPolicy::always(EventFault::CorruptedPayload);
    let mut corrupted_processor = ResilientEventProcessor::new(corrupted_policy, 3);
    let corrupted_res = corrupted_processor.process_event(event.clone());
    assert!(matches!(corrupted_res, Err(EventError::CorruptedPayload(_))));

    // Scenario 4: Deduplication of Repeated Events
    println!("[Reproduction Test] Testing deduplication of duplicate events...");
    let normal_policy = EventFaultPolicy::none();
    let mut dedup_processor = ResilientEventProcessor::new(normal_policy, 3);
    let first = dedup_processor.process_event(event.clone());
    assert!(first.is_ok());
    assert!(first.unwrap().is_some());
    assert_eq!(dedup_processor.processed_count(), 1);

    let second = dedup_processor.process_event(event.clone());
    assert!(second.is_ok());
    assert!(second.unwrap().is_none(), "Duplicate event must be discarded by deduplication");
    assert_eq!(dedup_processor.processed_count(), 1);

    // Scenario 5: Dropped Event Handling
    println!("[Reproduction Test] Testing dropped event handling...");
    let dropped_policy = EventFaultPolicy::always(EventFault::Dropped);
    let mut dropped_processor = ResilientEventProcessor::new(dropped_policy, 3);
    let dropped_res = dropped_processor.process_event(event);
    assert!(dropped_res.is_ok());
    assert_eq!(dropped_res.unwrap(), None);
    assert_eq!(dropped_processor.dropped_count(), 1);

    println!("[Reproduction Test] All chaos fault injection reproduction scenarios passed successfully!");
}

/// Reproduction test for TOPIC-20261105-781ba7 / TOPIC-20261105-1d2ebc:
/// Chaos Engineering and Fault Injection resilience in agent-domain::guardrail
#[test]
fn test_reproduce_and_verify_guardrail_chaos_resilience() {
    println!("[Reproduction Test] Initializing Guardrail Chaos Engineering & Fault Injection verification...");

    let policy = GuardrailPolicy::default_enterprise();

    // Verify simulate_fault on GuardrailPolicy
    assert!(matches!(
        policy.simulate_fault(&GuardrailFault::SocketTimeout),
        Err(GuardrailError::SocketTimeout)
    ));
    assert!(matches!(
        policy.simulate_fault(&GuardrailFault::ConnectionReset),
        Err(GuardrailError::ConnectionReset)
    ));
    assert!(matches!(
        policy.simulate_fault(&GuardrailFault::CorruptedRule),
        Err(GuardrailError::CorruptedPolicy(_))
    ));

    // Scenario 1: Intermittent socket timeout recovery
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::SocketTimeout, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy.clone(), fault_policy, 3);
    let mut ledger = AuditLedger::new();

    let res = evaluator.evaluate_and_record(&mut ledger, "git diff");
    assert!(res.is_ok(), "Guardrail evaluator must recover from intermittent socket timeout");
    assert!(evaluator.recovered_count() > 0);
    assert_eq!(evaluator.evaluated_count(), 1);
    assert!(ledger.verify_integrity().is_ok());

    // Scenario 2: Red line blocking under fault injection
    let res_blocked = evaluator.evaluate_and_record(&mut ledger, "DROP TABLE sensitive_data");
    assert!(matches!(res_blocked, Err(GuardrailError::Blocked(_))));
    assert_eq!(evaluator.blocked_count(), 1);
    assert!(ledger.verify_integrity().is_ok());

    // Scenario 3: Persistent fault fail-closed and exception audit recording
    let fail_policy = GuardrailFaultPolicy::always(GuardrailFault::ServiceUnavailable);
    let mut failing_evaluator = ResilientGuardrailEvaluator::new(policy, fail_policy, 2);
    let res_fail = failing_evaluator.evaluate_and_record(&mut ledger, "cargo build");
    assert!(matches!(res_fail, Err(GuardrailError::ServiceUnavailable(_))));
    assert_eq!(ledger.entries().last().unwrap().severity, ActionSeverity::HighImpact);
    assert!(ledger.verify_integrity().is_ok());

    println!("[Reproduction Test] Guardrail chaos fault injection reproduction verified successfully!");
}

/// Reproduction test for TOPIC-20261105-0ae8a5:
/// Identity early validation under persistent chaos & Guardrail variant detection
#[test]
fn test_reproduce_and_verify_identity_early_rejection_under_chaos() {
    use agent_domain::{
        AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
        IdentityError, IdentityFault, IdentityFaultPolicy, ResilientIdentityManager,
    };

    println!("[Reproduction Test] Initializing Identity early rejection under chaos...");

    let invalid_profile = AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::from_str(""),
            name: "   ".to_string(),
            description: "Invalid profile".to_string(),
            autonomy_level: AutonomyLevel::Supervised,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: "TEST".to_string(),
            title: "Tester".to_string(),
            primary_domain: "QA".to_string(),
            capabilities: vec![],
        },
        soul: AgentSoul {
            persona: "Tester".to_string(),
            tone_of_voice: "Neutral".to_string(),
            core_values: vec![],
            voice_profile: None,
        },
        mission: AgentMission {
            high_level_objective: "Verify early rejection".to_string(),
            kpis: vec![],
            constraints: vec![],
        },
    };

    // Even if fault policy is always SocketTimeout, invalid identity must be rejected immediately
    // without retry loops or counting recovery
    let fault_policy = IdentityFaultPolicy::always(IdentityFault::SocketTimeout);
    let mut manager = ResilientIdentityManager::new(fault_policy, 3);
    let mut ledger = AuditLedger::new();

    let res = manager.resolve_and_record(&mut ledger, invalid_profile);
    assert!(matches!(res, Err(IdentityError::InvalidIdentity(_))));
    assert_eq!(manager.recovered_count(), 0, "Must not consume retries or count recoveries for invalid identity");
    assert_eq!(manager.rejected_count(), 1);
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.entries()[0].action, "COMMAND_BLOCKED");
    assert!(ledger.verify_integrity().is_ok());

    println!("[Reproduction Test] Identity early rejection verified successfully!");
}

#[test]
fn test_reproduce_and_verify_guardrail_pattern_normalization_and_variants() {
    let policy = GuardrailPolicy::default_enterprise();

    // Verify variants of rm -rf and chmod
    assert!(policy.check_command_safety("rm -fr /data").is_err(), "Must block rm -fr");
    assert!(policy.check_command_safety("rm   -r   -f   /var").is_err(), "Must block rm -r -f");
    assert!(policy.check_command_safety("chmod 0777 /etc").is_err(), "Must block chmod 0777");
    assert!(policy.check_command_safety("chmod a+rwx /tmp").is_err(), "Must block chmod a+rwx");

    // Safe command must pass
    assert!(policy.check_command_safety("cargo check --release").is_ok());
}

/// Reproduction and verification test for TOPIC-20261106-c9a250:
/// Refactor unwrap() on task.find_subtask() to safe error handling mechanism (zero-panic).
#[test]
fn test_reproduce_and_verify_subtask_safe_lookup_and_panic_prevention() {
    println!("[Reproduction Test] Initializing Subtask Lookup & Panic Prevention verification...");

    let mut task = Task::new(
        "Autonomous Vulnerability Remediation",
        "Perform zero-panic subtask resolution across agent domain",
        Priority::High,
    );

    let subtask = SubTask::new(
        "sub-1",
        "Lint security rules",
        "cargo test -p agent-domain",
        "Exit code 0",
    );
    task.add_subtask(subtask);

    // Kịch bản 1: Tái hiện nguy cơ panic khi dùng unwrap() với subtask không tồn tại
    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = task.find_subtask("nonexistent").unwrap();
    }));
    assert!(
        panic_result.is_err(),
        "Calling .unwrap() on nonexistent subtask must panic in unhandled legacy code"
    );

    // Kịch bản 2: Cơ chế tra cứu an toàn get_subtask() trả về Ok khi subtask tồn tại
    let found = task.get_subtask("sub-1");
    assert!(found.is_ok(), "Existing subtask must resolve safely to Ok");
    assert_eq!(found.unwrap().status, TaskStatus::Queued);

    // Kịch bản 3: Cơ chế tra cứu an toàn get_subtask() trả về Err(TaskError::SubtaskNotFound) mà KHÔNG gây panic
    let not_found = task.get_subtask("nonexistent");
    assert!(
        matches!(not_found, Err(TaskError::SubtaskNotFound(ref id)) if id == "nonexistent"),
        "Missing subtask must return TaskError::SubtaskNotFound without panic"
    );

    // Kịch bản 4: Cơ chế tra cứu biến đổi an toàn get_subtask_mut() cho phép cập nhật trạng thái
    let mut_res = task.get_subtask_mut("sub-1");
    assert!(mut_res.is_ok(), "Existing subtask must resolve mutably to Ok");
    mut_res.unwrap().status = TaskStatus::Completed;

    // Xác nhận trạng thái đã được cập nhật thành công qua get_subtask
    let updated = task.get_subtask("sub-1");
    assert!(updated.is_ok());
    assert_eq!(updated.unwrap().status, TaskStatus::Completed);

    // Kịch bản 5: get_subtask_mut() trên ID không tồn tại trả về Err mà không hoảng loạn
    let mut_not_found = task.get_subtask_mut("nonexistent");
    assert!(
        matches!(mut_not_found, Err(TaskError::SubtaskNotFound(ref id)) if id == "nonexistent"),
        "Missing subtask on get_subtask_mut must return TaskError::SubtaskNotFound"
    );

    println!("[Reproduction Test] Subtask safe lookup and zero-panic verification passed successfully!");
}

/// TDD Reproduction and verification test for TOPIC-20261106-b15309 / TOPIC-20261106-036bf1:
/// Triệt tiêu lỗi hệ thống lặp lại (research_mission::TOPIC-20261106-036bf1) originating from
/// explicit `panic!()` in crates/agent-domain/src/task.rs and recurring mission cascades.
///
/// Verifies:
/// 1. Source verification: crates/agent-domain/src/task.rs must contain ZERO explicit `panic!()` calls.
/// 2. Functional verification: querying, updating, or failing to find subtasks in `Task` must never panic.
/// 3. Error propagation: missing subtasks return structured `TaskError::SubtaskNotFound`.
#[test]
fn test_reproduce_and_verify_topic_b15309_zero_panic_in_task_rs() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let task_rs_path = manifest_dir.join("src/task.rs");
    assert!(task_rs_path.is_file(), "task.rs must exist at {}", task_rs_path.display());

    let content = std::fs::read_to_string(&task_rs_path).expect("Failed to read src/task.rs");

    // Scan for any explicit panic!() call in task.rs
    let panic_matches: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim();
            !trimmed.starts_with("//") && trimmed.contains("panic!(")
        })
        .map(|(idx, line)| (idx + 1, line.trim()))
        .collect();

    assert!(
        panic_matches.is_empty(),
        "Detected explicit panic!() in src/task.rs (TOPIC-20261106-069a77 / TOPIC-20261106-b15309 regression):\n{:#?}",
        panic_matches
    );

    // Functional verification: Task with mixed subtasks operates without panic
    let mut task = Task::new(
        "Self-Healing Lifecycle Task",
        "Verify zero-panic resilience",
        Priority::High,
    );

    // 1. Empty lookup is safe
    assert!(task.get_subtask("st-absent").is_err());
    assert_eq!(task.find_subtask("st-absent"), None);

    // 2. Add and mutate subtasks safely
    let sub = SubTask::new("st-1", "Step 1", "echo ok", "Exit code 0");
    task.add_subtask(sub);

    let retrieved = task.get_subtask("st-1");
    assert!(retrieved.is_ok());
    assert_eq!(retrieved.unwrap().id, "st-1");

    let mutated = task.get_subtask_mut("st-1");
    assert!(mutated.is_ok());
    mutated.unwrap().status = TaskStatus::Completed;

    assert_eq!(
        task.get_subtask("st-1").map(|s| &s.status),
        Ok(&TaskStatus::Completed)
    );
    assert_eq!(
        task.find_subtask("st-1").map(|s| &s.status),
        Some(&TaskStatus::Completed)
    );
}


