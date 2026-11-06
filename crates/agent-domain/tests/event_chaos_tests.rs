use agent_domain::{
    ActionSeverity, AgentId, AuditEventType, AuditLedger, EventBenchmarkHarness,
    EventBenchmarkStats, EventError, EventFault, EventFaultPolicy, InterAgentEvent,
    ResilientEventProcessor,
};

#[test]
fn test_event_validation_and_integrity() {
    let agent_a = AgentId::new("researcher-01");
    let agent_b = AgentId::new("coder-02");
    let valid_event = InterAgentEvent::new(
        agent_a.clone(),
        Some(agent_b.clone()),
        "task.delegated",
        serde_json::json!({ "subtask_id": "sub-1", "instructions": "Run analysis" }),
    );

    assert!(valid_event.validate().is_ok());

    let mut invalid_event = valid_event.clone();
    invalid_event.id = "INVALID_NO_PREFIX".to_string();
    assert!(matches!(
        invalid_event.validate(),
        Err(EventError::InvalidStructure(_))
    ));

    let mut invalid_spec = valid_event.clone();
    invalid_spec.specversion = "0.1".to_string();
    assert!(matches!(
        invalid_spec.validate(),
        Err(EventError::InvalidStructure(_))
    ));
}

#[test]
fn test_intermittent_socket_timeout_recovery() {
    let agent_a = AgentId::new("agent-alpha");
    let agent_b = AgentId::new("agent-beta");
    let event = InterAgentEvent::new(
        agent_a,
        Some(agent_b),
        "heartbeat.ping",
        serde_json::json!({ "sequence": 1 }),
    );

    // Fail every 2nd attempt with SocketTimeout, max retries 3
    let policy = EventFaultPolicy::intermittent(EventFault::SocketTimeout, 2);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    let result = processor.process_event(event);
    assert!(result.is_ok());
    let dispatched = result.unwrap();
    assert!(dispatched.is_some());
    assert!(processor.recovered_count() > 0);
    assert_eq!(processor.processed_count(), 1);
}

#[test]
fn test_max_retries_exceeded_on_persistent_timeout() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "broadcast.alert",
        serde_json::json!({ "alert": "cpu_spike" }),
    );

    // Always inject SocketTimeout, max retries 2
    let policy = EventFaultPolicy::always(EventFault::SocketTimeout);
    let mut processor = ResilientEventProcessor::new(policy, 2);

    let result = processor.process_event(event);
    assert_eq!(result, Err(EventError::SocketTimeout));
    assert_eq!(processor.processed_count(), 0);
}

#[test]
fn test_connection_reset_recovery() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "io.sync",
        serde_json::json!({ "chunk": 42 }),
    );

    // Inject connection reset intermittent with max retries 3
    let policy = EventFaultPolicy::intermittent(EventFault::ConnectionReset, 2);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    let result = processor.process_event(event);
    assert!(result.is_ok());
    assert!(processor.recovered_count() > 0);
}

#[test]
fn test_corrupted_payload_detection_and_rejection() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "data.payload",
        serde_json::json!({ "secure_data": "secret_token_123" }),
    );

    let policy = EventFaultPolicy::always(EventFault::CorruptedPayload);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    let result = processor.process_event(event);
    assert!(matches!(result, Err(EventError::CorruptedPayload(_))));
}

#[test]
fn test_dropped_event_handling() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "telemetry.metric",
        serde_json::json!({ "metric": "ram_usage", "val": 84 }),
    );

    let policy = EventFaultPolicy::always(EventFault::Dropped);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    let result = processor.process_event(event);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), None);
    assert_eq!(processor.dropped_count(), 1);
}

#[test]
fn test_duplicate_event_deduplication() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "payment.charge",
        serde_json::json!({ "tx_id": "tx_999", "amount": 100 }),
    );

    let policy = EventFaultPolicy::none();
    let mut processor = ResilientEventProcessor::new(policy, 3);

    // First attempt: processed
    let first = processor.process_event(event.clone()).unwrap();
    assert!(first.is_some());
    assert_eq!(processor.processed_count(), 1);

    // Duplicate attempt with same event ID: suppressed safely
    let second = processor.process_event(event).unwrap();
    assert_eq!(second, None);
    assert_eq!(processor.processed_count(), 1);
}

#[test]
fn test_high_volume_chaos_resilience() {
    let agent_a = AgentId::new("worker-pool");
    let policy = EventFaultPolicy::intermittent(EventFault::SocketTimeout, 3);
    let mut processor = ResilientEventProcessor::new(policy, 4);

    let mut successful = 0;
    for i in 0..50 {
        let event = InterAgentEvent::new(
            agent_a.clone(),
            None,
            "batch.step",
            serde_json::json!({ "index": i }),
        );

        if let Ok(Some(_)) = processor.process_event(event) {
            successful += 1;
        }
    }

    assert_eq!(successful, 50);
    assert!(processor.recovered_count() > 0);
}

#[test]
fn test_max_retries_exceeded_on_persistent_connection_reset() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "sync.state",
        serde_json::json!({ "sequence": 99 }),
    );

    // Always inject ConnectionReset, max retries 2
    let policy = EventFaultPolicy::always(EventFault::ConnectionReset);
    let mut processor = ResilientEventProcessor::new(policy, 2);

    let result = processor.process_event(event);
    assert_eq!(result, Err(EventError::ConnectionReset));
    assert_eq!(processor.processed_count(), 0);
}

#[test]
fn test_processor_early_rejection_on_invalid_event() {
    let agent_a = AgentId::new("agent-alpha");
    let mut invalid_event = InterAgentEvent::new(
        agent_a,
        None,
        "dummy",
        serde_json::json!({ "malformed": true }),
    );
    invalid_event.event_type = "   ".to_string();

    let policy = EventFaultPolicy::always(EventFault::SocketTimeout);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    // Should fail validation before fault injection or retry loop
    let result = processor.process_event(invalid_event);
    assert!(matches!(result, Err(EventError::InvalidStructure(_))));
    assert_eq!(processor.processed_count(), 0);
    assert_eq!(processor.recovered_count(), 0);
}

#[test]
fn test_simulate_fault_direct_failure_scenarios() {
    let agent = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent,
        None,
        "diagnostic.ping",
        serde_json::json!({ "test": true }),
    );

    assert_eq!(
        event.simulate_fault(&EventFault::SocketTimeout),
        Err(EventError::SocketTimeout)
    );
    assert_eq!(
        event.simulate_fault(&EventFault::ConnectionReset),
        Err(EventError::ConnectionReset)
    );
    assert_eq!(
        event.simulate_fault(&EventFault::Dropped),
        Err(EventError::Dropped)
    );
    assert!(matches!(
        event.simulate_fault(&EventFault::CorruptedPayload),
        Err(EventError::CorruptedPayload(_))
    ));
    assert_eq!(
        event.simulate_fault(&EventFault::Duplicate),
        Ok(event.clone())
    );
}

#[test]
fn test_recurrent_failure_mitigation() {
    let sender = AgentId::new("agent-coordinator");
    let receiver = AgentId::new("agent-executor");

    // Scenario 1: Recurrent transient socket timeouts with eventual recovery
    {
        // Fail on 1st injection, succeed on retry (max_injections = 2, max_retries = 3)
        let policy = EventFaultPolicy::intermittent(EventFault::SocketTimeout, 1).with_max_injections(2);
        let mut processor = ResilientEventProcessor::new(policy, 3);

        let event = InterAgentEvent::new(
            sender.clone(),
            Some(receiver.clone()),
            "task.dispatch",
            serde_json::json!({ "mission_id": "MISSION-001", "retry_policy": "exponential" }),
        );

        let result = processor.process_event(event.clone());
        assert!(result.is_ok(), "Expected recovery from recurrent socket timeouts");
        let processed_event = result.unwrap();
        assert!(processed_event.is_some());
        assert_eq!(processed_event.unwrap().id, event.id);
        assert_eq!(processor.processed_count(), 1);
        assert_eq!(processor.recovered_count(), 2);
    }

    // Scenario 2: Recurrent connection resets with resilient retry
    {
        let policy = EventFaultPolicy::intermittent(EventFault::ConnectionReset, 1).with_max_injections(1);
        let mut processor = ResilientEventProcessor::new(policy, 3);

        let event = InterAgentEvent::new(
            sender.clone(),
            Some(receiver.clone()),
            "heartbeat.sync",
            serde_json::json!({ "status": "healthy" }),
        );

        let result = processor.process_event(event);
        assert!(result.is_ok(), "Processor must mitigate recurrent connection reset");
        assert_eq!(processor.processed_count(), 1);
        assert_eq!(processor.recovered_count(), 1);
    }

    // Scenario 3: Recurrent duplicate event storms - deduplication mitigation
    {
        let policy = EventFaultPolicy::none();
        let mut processor = ResilientEventProcessor::new(policy, 3);

        let event = InterAgentEvent::new(
            sender.clone(),
            Some(receiver.clone()),
            "state.update",
            serde_json::json!({ "step": 42 }),
        );

        // First delivery: processes successfully
        let res1 = processor.process_event(event.clone()).expect("First delivery must succeed");
        assert!(res1.is_some());
        assert_eq!(processor.processed_count(), 1);

        // Recurrent duplicate deliveries: safely suppressed by deduplication
        for _ in 0..5 {
            let res_dup = processor.process_event(event.clone()).expect("Duplicate must be safely handled");
            assert!(res_dup.is_none(), "Duplicate event must be ignored");
        }
        assert_eq!(processor.processed_count(), 1, "Only initial event should increment processed count");
    }

    // Scenario 4: Persistent failure exceeding max retries is cleanly reported without panic
    {
        let policy = EventFaultPolicy::always(EventFault::SocketTimeout);
        let mut processor = ResilientEventProcessor::new(policy, 2);

        let event = InterAgentEvent::new(
            sender.clone(),
            Some(receiver.clone()),
            "critical.alert",
            serde_json::json!({ "level": "fatal" }),
        );

        let result = processor.process_event(event);
        assert_eq!(result, Err(EventError::SocketTimeout));
        assert_eq!(processor.processed_count(), 0);
        assert_eq!(processor.recovered_count(), 2);
    }

    // Scenario 5: Resetting processor state enables clean recovery for subsequent runs
    {
        let policy = EventFaultPolicy::none();
        let mut processor = ResilientEventProcessor::new(policy, 3);

        let event = InterAgentEvent::new(
            sender,
            Some(receiver),
            "lifecycle.restart",
            serde_json::json!({ "epoch": 1 }),
        );

        let _ = processor.process_event(event.clone()).unwrap();
        assert_eq!(processor.processed_count(), 1);

        processor.clear_seen_ids();
        processor.reset_counts();
        assert_eq!(processor.processed_count(), 0);

        let res_reprocessed = processor.process_event(event).unwrap();
        assert!(res_reprocessed.is_some());
        assert_eq!(processor.processed_count(), 1);
    }
}

#[test]
fn test_transient_io_failure_recovery() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "telemetry.io",
        serde_json::json!({ "metric": "disk_io", "value": 1024 }),
    );

    // Fail every 2nd attempt with TransientIo, max retries 3
    let policy = EventFaultPolicy::intermittent(EventFault::TransientIo, 2);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    let result = processor.process_event(event);
    assert!(result.is_ok());
    assert_eq!(processor.processed_count(), 1);
    assert!(processor.recovered_count() > 0);
}

#[test]
fn test_service_unavailable_recovery() {
    let agent_a = AgentId::new("agent-alpha");
    let event = InterAgentEvent::new(
        agent_a,
        None,
        "service.ping",
        serde_json::json!({ "ping": true }),
    );

    // Intermittent service unavailable every 2 attempts, max retries 3
    let policy = EventFaultPolicy::intermittent(EventFault::ServiceUnavailable, 2);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    let result = processor.process_event(event);
    assert!(result.is_ok());
    assert_eq!(processor.processed_count(), 1);
    assert!(processor.recovered_count() > 0);
}

#[test]
fn test_event_audit_ledger_chaos_integration() {
    let agent_a = AgentId::new("agent-sender");
    let agent_b = AgentId::new("agent-receiver");
    let mut ledger = AuditLedger::new();

    // 1. Intermittent socket timeout with successful recovery & audit entry
    let policy = EventFaultPolicy::intermittent(EventFault::SocketTimeout, 2);
    let mut processor = ResilientEventProcessor::new(policy, 3);

    let event1 = InterAgentEvent::new(
        agent_a.clone(),
        Some(agent_b.clone()),
        "task.start",
        serde_json::json!({ "step": 1 }),
    );

    let res1 = processor.process_and_record(&mut ledger, event1.clone());
    assert!(res1.is_ok());
    assert_eq!(processor.processed_count(), 1);
    assert!(processor.recovered_count() > 0);

    // 2. Duplicate event storm under audit ledger
    let res_dup = processor.process_and_record(&mut ledger, event1);
    assert!(res_dup.is_ok());
    assert_eq!(res_dup.unwrap(), None);

    // 3. Corrupted payload rejection under audit ledger
    let mut fail_processor = ResilientEventProcessor::new(
        EventFaultPolicy::always(EventFault::CorruptedPayload),
        2,
    );
    let event2 = InterAgentEvent::new(
        agent_a.clone(),
        Some(agent_b.clone()),
        "task.corrupted",
        serde_json::json!({ "invalid": true }),
    );
    let res_corrupted = fail_processor.process_and_record(&mut ledger, event2);
    assert!(matches!(res_corrupted, Err(EventError::CorruptedPayload(_))));
    assert_eq!(fail_processor.rejected_count(), 1);

    // 4. Persistent failure exceeding retries under audit ledger
    let mut timeout_processor = ResilientEventProcessor::new(
        EventFaultPolicy::always(EventFault::SocketTimeout),
        2,
    );
    let event3 = InterAgentEvent::new(
        agent_a,
        Some(agent_b),
        "task.timeout",
        serde_json::json!({ "timeout": true }),
    );
    let res_timeout = timeout_processor.process_and_record(&mut ledger, event3);
    assert_eq!(res_timeout, Err(EventError::SocketTimeout));

    // Verify cryptographic audit ledger integrity
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.len(), 4);
    assert_eq!(ledger.entries()[0].action, AuditEventType::CommandAllowed.as_str());
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::Routine);
    assert_eq!(ledger.entries()[1].action, AuditEventType::CommandAllowed.as_str());
    assert_eq!(ledger.entries()[1].severity, ActionSeverity::Routine);
    assert_eq!(ledger.entries()[2].action, AuditEventType::CommandException.as_str());
    assert_eq!(ledger.entries()[2].severity, ActionSeverity::HighImpact);
    assert_eq!(ledger.entries()[3].action, AuditEventType::CommandException.as_str());
    assert_eq!(ledger.entries()[3].severity, ActionSeverity::HighImpact);
}

#[test]
fn test_event_fault_to_error_conversion() {
    assert_eq!(EventFault::SocketTimeout.to_error(), EventError::SocketTimeout);
    assert_eq!(EventFault::ConnectionReset.to_error(), EventError::ConnectionReset);
    assert!(matches!(
        EventFault::TransientIo.to_error(),
        EventError::TransientIo(_)
    ));
    assert!(matches!(
        EventFault::ServiceUnavailable.to_error(),
        EventError::ServiceUnavailable(_)
    ));
    assert!(matches!(
        EventFault::CorruptedPayload.to_error(),
        EventError::CorruptedPayload(_)
    ));
    assert_eq!(EventFault::Dropped.to_error(), EventError::Dropped);
}

// =========================================================================
// Root Cause Elimination Test for TOPIC-20261105-52d2a0 & TOPIC-20261105-243a67
// High Load Benchmark, P99 Latency & Chaos Resilience for agent-domain::event
// =========================================================================

#[test]
fn test_topic_52d2a0_243a67_event_benchmark_and_resilience() {
    // 1. Verify LOC compliance across all modular event components (< 350 lines P90)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let event_dir = manifest_dir.join("src/event");
    assert!(
        event_dir.is_dir(),
        "src/event must exist as a modular directory"
    );

    let event_root = manifest_dir.join("src/event.rs");
    if event_root.is_file() {
        let content = std::fs::read_to_string(&event_root).unwrap();
        let lines = content.lines().count();
        assert!(
            lines <= 350,
            "event.rs has {} lines, which exceeds P90 limit (350 lines)",
            lines
        );
    }

    for entry in std::fs::read_dir(&event_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let content = std::fs::read_to_string(&path).unwrap();
            let lines = content.lines().count();
            assert!(
                lines <= 350,
                "Submodule {} has {} lines, which exceeds P90 limit (350 lines)",
                path.file_name().unwrap().to_str().unwrap(),
                lines
            );
        }
    }

    // 2. High-load benchmark: 10,000 requests measuring P99 latency and proving bounded memory allocation
    let mut processor = ResilientEventProcessor::new(EventFaultPolicy::none(), 3)
        .with_max_capacity(10_000);

    let stats: EventBenchmarkStats = EventBenchmarkHarness::run_benchmark(&mut processor, 10_000);
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_processed, 10_000);
    assert!(
        stats.throughput_ops_per_sec > 1_000.0,
        "Throughput should exceed 1000 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(
        stats.p99_latency_us < 5_000.0,
        "P99 latency should be under 5000 us (5ms), got {} us",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // Verify bounded memory allocation (no unbounded leak):
    let mem_before = processor.memory_usage_bytes();
    // Run another 5,000 requests to trigger eviction in bounded capacity
    let _ = EventBenchmarkHarness::run_benchmark(&mut processor, 5_000);
    let mem_after = processor.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 1024,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before, mem_after
    );

    // 3. Batch processing optimization test
    let sender = AgentId::new("batch-sender");
    let batch_events: Vec<InterAgentEvent> = (0..100)
        .map(|i| {
            InterAgentEvent::new(
                sender.clone(),
                None,
                "batch.test",
                serde_json::json!({ "idx": i }),
            )
        })
        .collect();

    let batch_res = processor.process_batch(batch_events).expect("batch process succeeds");
    assert_eq!(batch_res.len(), 100);

    // 4. Concurrent benchmark test (10,000 requests across 4 threads)
    let concurrent_stats = EventBenchmarkHarness::run_concurrent_benchmark(10_000, 4);
    assert_eq!(concurrent_stats.total_requests, 10_000);
    assert_eq!(concurrent_stats.successful_processed, 10_000);
    assert!(
        concurrent_stats.throughput_ops_per_sec > 500.0,
        "Concurrent throughput should be healthy, got {}",
        concurrent_stats.throughput_ops_per_sec
    );
    assert!(
        concurrent_stats.p99_latency_us < 20_000.0,
        "Concurrent P99 latency should be well under 20ms, got {} us",
        concurrent_stats.p99_latency_us
    );
}



