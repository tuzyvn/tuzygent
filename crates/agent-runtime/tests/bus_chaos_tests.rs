use agent_domain::{ActionSeverity, AgentId, AuditEventType, AuditLedger, InterAgentEvent};
use agent_runtime::{
    simulate_bus_fault, BusError, BusFault, BusFaultPolicy, InterAgentBus, ResilientInterAgentBus,
    ResilientSubscriber,
};

fn create_sample_event(source: &str, target: Option<&str>, event_type: &str) -> InterAgentEvent {
    InterAgentEvent::new(
        AgentId::new(source),
        target.map(AgentId::new),
        event_type,
        serde_json::json!({
            "correlation_id": uuid::Uuid::new_v4().to_string(),
            "payload": "chaos engineering resilience payload"
        }),
    )
}

#[test]
fn test_bus_fault_simulation_direct() {
    let faults = vec![
        (BusFault::SocketTimeout, BusError::SocketTimeout),
        (BusFault::ConnectionReset, BusError::ConnectionReset),
        (BusFault::ChannelClosed, BusError::ChannelClosed),
    ];

    for (fault, expected_err) in faults {
        let err = fault.to_error();
        assert_eq!(err, expected_err);
        assert_eq!(simulate_bus_fault(&fault).unwrap_err(), expected_err);
    }
}

#[tokio::test]
async fn test_intermittent_socket_timeout_recovery() {
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::intermittent(BusFault::SocketTimeout, 2),
        3,
    );
    let mut rx = bus.subscribe();

    let event = create_sample_event("supervisor", Some("worker-1"), "task.dispatch");
    let result = bus.publish_resilient(event.clone());

    assert!(result.is_ok(), "Expected success after retry recovery: {:?}", result.err());
    assert!(bus.recovered_count() > 0);
    assert_eq!(bus.published_count(), 1);

    // Verify receiver got the event
    let received = rx.recv().await.expect("Subscriber should receive event");
    assert_eq!(received.id, event.id);
}

#[tokio::test]
async fn test_intermittent_connection_reset_recovery() {
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::intermittent(BusFault::ConnectionReset, 2),
        3,
    );
    let mut rx = bus.subscribe();

    let event = create_sample_event("worker-1", None, "worker.heartbeat");
    let result = bus.publish_resilient(event.clone());

    assert!(result.is_ok(), "Expected recovery from connection reset");
    assert!(bus.recovered_count() > 0);
    assert_eq!(bus.published_count(), 1);

    let received = rx.recv().await.expect("Subscriber should receive heartbeat");
    assert_eq!(received.id, event.id);
}

#[tokio::test]
async fn test_transient_io_failure_recovery() {
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::intermittent(BusFault::TransientIo, 2),
        4,
    );

    let event = create_sample_event("watchdog", Some("supervisor"), "alert.high_memory");
    let result = bus.publish_resilient(event);

    assert!(result.is_ok());
    assert!(bus.recovered_count() > 0);
}

#[tokio::test]
async fn test_service_unavailable_recovery() {
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::intermittent(BusFault::ServiceUnavailable, 2),
        3,
    );

    let event = create_sample_event("engine", None, "system.maintenance");
    let result = bus.publish_resilient(event);

    assert!(result.is_ok());
    assert!(bus.recovered_count() > 0);
}

#[tokio::test]
async fn test_intermittent_channel_full_recovery() {
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::intermittent(BusFault::ChannelFull, 2),
        3,
    );
    let mut rx = bus.subscribe();

    let event = create_sample_event("producer", Some("consumer"), "buffer.test");
    let result = bus.publish_resilient(event.clone());

    assert!(result.is_ok(), "Expected recovery from intermittent channel capacity overflow");
    assert!(bus.recovered_count() > 0);
    assert_eq!(bus.published_count(), 1);

    let received = rx.recv().await.expect("Subscriber should receive event");
    assert_eq!(received.id, event.id);
}

#[tokio::test]
async fn test_broadcast_buffer_overflow_and_subscriber_lagged_recovery() {
    // Channel with small capacity 4
    let capacity = 4;
    let bus = InterAgentBus::new(capacity);
    let mut rx = bus.subscribe();

    // Publish 10 messages without reading, overflowing the buffer capacity
    let total_messages = 10;
    for i in 0..total_messages {
        let event = create_sample_event("sender", None, &format!("overflow.msg.{}", i));
        let res = bus.publish(event);
        assert!(res.is_ok());
    }

    // Subscriber should detect buffer capacity overflow (Lagged error)
    match rx.recv().await {
        Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
            assert!(missed > 0, "Subscriber must detect missed messages due to buffer overflow");
        }
        other => panic!("Expected RecvError::Lagged due to buffer overflow, got {:?}", other),
    }

    // Subsequent recv should recover and receive remaining recent messages
    let recovered_msg = rx.recv().await.expect("Subscriber should recover after lagging");
    assert!(recovered_msg.event_type.starts_with("overflow.msg."));
}

#[tokio::test]
async fn test_dropped_subscribers_and_recovery() {
    let mut bus = ResilientInterAgentBus::new(
        16,
        BusFaultPolicy::none(),
        3,
    );

    // Initial subscriber
    let rx1 = bus.subscribe();
    assert_eq!(bus.receiver_count(), 1);

    let event1 = create_sample_event("source-1", None, "event.first");
    let res1 = bus.publish_resilient(event1);
    assert_eq!(res1.unwrap(), 1);

    // Drop the only subscriber
    drop(rx1);
    assert_eq!(bus.receiver_count(), 0);

    // Publishing to dropped subscribers should succeed gracefully with 0 recipients
    let event2 = create_sample_event("source-1", None, "event.orphaned");
    let res2 = bus.publish_resilient(event2);
    assert_eq!(res2.unwrap(), 0, "Must safely deliver with 0 active recipients when subscribers drop");

    // Reconnection / recovery: new subscriber attaches
    let mut rx2 = bus.subscribe();
    assert_eq!(bus.receiver_count(), 1);

    let event3 = create_sample_event("source-1", None, "event.recovered");
    let res3 = bus.publish_resilient(event3.clone());
    assert_eq!(res3.unwrap(), 1);

    let received = rx2.recv().await.expect("New subscriber must receive post-recovery event");
    assert_eq!(received.id, event3.id);
}

#[tokio::test]
async fn test_partial_subscriber_drop_resilience() {
    let mut bus = ResilientInterAgentBus::new(
        16,
        BusFaultPolicy::none(),
        3,
    );

    let rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();
    let rx3 = bus.subscribe();
    assert_eq!(bus.receiver_count(), 3);

    // Drop two of the three subscribers
    drop(rx1);
    drop(rx3);
    assert_eq!(bus.receiver_count(), 1);

    // Dispatch message; surviving subscriber receives it uninterrupted
    let event = create_sample_event("source-x", None, "event.partial_drop");
    let res = bus.publish_resilient(event.clone());
    assert_eq!(res.unwrap(), 1);

    let received = rx2.recv().await.expect("Surviving subscriber must receive event");
    assert_eq!(received.id, event.id);
}

#[tokio::test]
async fn test_max_retries_exceeded_on_persistent_timeout() {
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::always(BusFault::SocketTimeout),
        3,
    );

    let event = create_sample_event("agent-a", Some("agent-b"), "data.sync");
    let result = bus.publish_resilient(event);

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), BusError::MaxRetriesExceeded(3));
    assert_eq!(bus.failed_count(), 1);
    assert_eq!(bus.published_count(), 0);
}

#[tokio::test]
async fn test_corrupted_event_rejection() {
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::always(BusFault::CorruptedEvent),
        3,
    );

    let event = create_sample_event("attacker", None, "malformed.packet");
    let result = bus.publish_resilient(event);

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BusError::CorruptedEvent(_)));
    assert_eq!(bus.rejected_count(), 1);
    assert_eq!(bus.recovered_count(), 0);
}

#[tokio::test]
async fn test_bus_audit_ledger_chaos_integration() {
    let mut ledger = AuditLedger::new();

    // 1. Successful publish under intermittent fault
    let mut resilient_bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::intermittent(BusFault::SocketTimeout, 2),
        3,
    );
    let _rx = resilient_bus.subscribe();

    let ok_event = create_sample_event("agent-1", Some("agent-2"), "task.ack");
    let res = resilient_bus.publish_and_record(&mut ledger, ok_event);
    assert!(res.is_ok());

    // 2. Rejected publish under corrupted event fault
    let mut corrupt_bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::always(BusFault::CorruptedEvent),
        2,
    );
    let bad_event = create_sample_event("unknown", None, "bad.signal");
    let err_res = corrupt_bus.publish_and_record(&mut ledger, bad_event);
    assert!(err_res.is_err());

    // 3. Verify ledger integrity and recorded entries
    assert_eq!(ledger.len(), 2);
    assert!(ledger.verify_integrity().is_ok(), "Audit ledger hash chain must remain valid");

    let entries = ledger.entries();
    assert_eq!(entries[0].event_type, AuditEventType::CommandAllowed);
    assert_eq!(entries[0].severity, ActionSeverity::Routine);

    assert_eq!(entries[1].event_type, AuditEventType::CommandBlocked);
    assert_eq!(entries[1].severity, ActionSeverity::Destructive);
}

#[tokio::test]
async fn test_high_volume_bus_chaos_resilience() {
    let mut bus = ResilientInterAgentBus::new(
        128,
        BusFaultPolicy::intermittent(BusFault::TransientIo, 3),
        5,
    );
    let mut rx = bus.subscribe();

    let count = 50;
    for i in 0..count {
        let event = create_sample_event("stress-tester", None, &format!("batch.msg.{}", i));
        let res = bus.publish_resilient(event);
        assert!(res.is_ok(), "Message {} must be safely delivered: {:?}", i, res.err());
    }

    assert_eq!(bus.published_count(), count);
    assert!(bus.recovered_count() > 0);

    for i in 0..count {
        let received = rx.recv().await.expect("Must receive all sent messages");
        assert_eq!(received.event_type, format!("batch.msg.{}", i));
    }
}

/// TDD test case for TOPIC-20261105-ff99fb & TOPIC-20261105-31d780:
/// Chaos Engineering and Fault Injection resilience in agent-runtime::bus.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-31d780.
#[tokio::test]
async fn test_topic_ff99fb_31d780_bus_chaos_resilience() {
    let base_bus = InterAgentBus::new(32);
    let mut rx = base_bus.subscribe();

    let policy = BusFaultPolicy::intermittent(BusFault::SocketTimeout, 2).with_max_injections(4);
    let mut resilient_bus = ResilientInterAgentBus::from_bus(base_bus, policy, 3);

    let event = create_sample_event("coordinator", Some("executor"), "mission.execute");
    let res = resilient_bus.publish_resilient(event.clone());

    assert!(res.is_ok(), "Must survive intermittent fault injection without failing mission");
    assert_eq!(resilient_bus.published_count(), 1);
    assert!(resilient_bus.recovered_count() > 0);

    let msg = rx.recv().await.expect("Subscriber on base bus must receive the dispatched event");
    assert_eq!(msg.id, event.id);
}

#[tokio::test]
async fn test_bus_payload_overload_protection() {
    let bus = InterAgentBus::new(32);
    let _rx = bus.subscribe();

    // Set max payload size limit to 50 bytes
    bus.set_max_payload_bytes(Some(50));

    // Event with small payload fits limit
    let small_event = InterAgentEvent::new(
        AgentId::new("agent-1"),
        None,
        "small.event",
        serde_json::json!({"ok": true}),
    );
    let res_small = bus.safe_publish(small_event);
    assert!(res_small.is_ok());

    // Event with large payload exceeding 50 bytes should be rejected with CapacityOverload
    let large_event = InterAgentEvent::new(
        AgentId::new("agent-1"),
        None,
        "large.event",
        serde_json::json!({
            "huge_field": "012345678901234567890123456789012345678901234567890123456789"
        }),
    );
    let res_large = bus.safe_publish(large_event);
    assert!(res_large.is_err());
    match res_large.unwrap_err() {
        BusError::CapacityOverload(msg) => {
            assert!(msg.contains("payload size"));
        }
        other => panic!("Expected BusError::CapacityOverload, got {:?}", other),
    }
}

#[tokio::test]
async fn test_bus_watermark_overload_protection() {
    let bus = InterAgentBus::new(32);
    let _rx1 = bus.subscribe();
    let _rx2 = bus.subscribe();
    let _rx3 = bus.subscribe();

    // Watermark threshold: 2 subscribers max
    bus.set_overload_watermark(Some(2));

    let event = create_sample_event("sender", None, "test.watermark");
    let res = bus.safe_publish(event);
    assert!(res.is_err());
    match res.unwrap_err() {
        BusError::CapacityOverload(msg) => {
            assert!(msg.contains("receiver count 3 exceeds watermark 2"));
        }
        other => panic!("Expected BusError::CapacityOverload, got {:?}", other),
    }
}

#[tokio::test]
async fn test_resilient_subscriber_auto_catchup_on_overflow() {
    let bus = InterAgentBus::new(4);
    let mut resilient_rx: ResilientSubscriber = bus.subscribe_resilient();

    // Publish 10 messages to cause buffer overflow
    for i in 0..10 {
        let event = create_sample_event("sender", None, &format!("overflow.event.{}", i));
        let res = bus.publish(event);
        assert!(res.is_ok());
    }

    // Resilient subscriber catches up automatically without panicking
    let caught_event = resilient_rx
        .recv_resilient()
        .await
        .expect("Should safely catch up to fresh event without panicking");
    assert!(resilient_rx.total_lagged() > 0, "Must track lagged message count");
    assert!(caught_event.event_type.starts_with("overflow.event."));
}

#[tokio::test]
async fn test_bus_manual_reconstitution_recovery() {
    let bus = InterAgentBus::new(16);
    let _rx = bus.subscribe();

    // Close the bus
    bus.close();
    assert!(bus.is_closed());

    let event = create_sample_event("agent", None, "post.close");
    let res_closed = bus.safe_publish(event.clone());
    assert_eq!(res_closed.unwrap_err(), BusError::ChannelClosed);

    // Reconstitute bus with new capacity 32
    let new_cap = bus.reconstitute(Some(32));
    assert_eq!(new_cap, 32);
    assert_eq!(bus.capacity(), 32);
    assert!(!bus.is_closed());

    // Subscribe to reconstituted bus and publish
    let mut new_rx = bus.subscribe();
    let res_reconstituted = bus.safe_publish(event.clone());
    assert!(res_reconstituted.is_ok());

    let received = new_rx.recv().await.expect("Must receive on reconstituted bus");
    assert_eq!(received.id, event.id);
}

#[tokio::test]
async fn test_resilient_bus_auto_reconstitution_on_channel_closed() {
    let mut bus = ResilientInterAgentBus::new(16, BusFaultPolicy::none(), 3);
    let mut rx = bus.subscribe();

    // First event succeeds
    let event1 = create_sample_event("agent-1", None, "before.close");
    let res1 = bus.publish_resilient(event1.clone());
    assert_eq!(res1.unwrap(), 1);
    let _ = rx.recv().await.unwrap();

    // Explicitly close underlying bus
    bus.bus().close();
    assert!(bus.bus().is_closed());

    // Publish with auto-reconstitution enabled: recovers automatically!
    let event2 = create_sample_event("agent-1", None, "after.reconstitution");
    let res2 = bus.publish_resilient(event2.clone());
    assert!(res2.is_ok(), "Must auto-reconstitute closed bus and succeed: {:?}", res2.err());
    assert!(bus.reconstituted_count() > 0);
    assert!(!bus.bus().is_closed());
}

#[tokio::test]
async fn test_bus_fault_panic_resilience_and_protection() {
    // Intermittent panic fault: should be caught, recovered via retry without crashing thread
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::intermittent(BusFault::Panic, 2),
        3,
    );
    let mut rx = bus.subscribe();

    let event = create_sample_event("tester", None, "panic.resilience");
    let res = bus.publish_resilient(event.clone());

    assert!(res.is_ok(), "Intermittent panic must be recovered safely");
    assert!(bus.recovered_count() > 0);
    assert_eq!(bus.published_count(), 1);

    let received = rx.recv().await.expect("Subscriber receives message");
    assert_eq!(received.id, event.id);
}

#[tokio::test]
async fn test_publish_and_record_panic_safety() {
    let mut ledger = AuditLedger::new();
    let mut bus = ResilientInterAgentBus::new(
        64,
        BusFaultPolicy::always(BusFault::Panic),
        2,
    );

    let event = create_sample_event("agent", None, "persistent.panic");
    let res = bus.publish_and_record(&mut ledger, event);

    // Max retries exceeded due to persistent panic; process did not crash
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), BusError::MaxRetriesExceeded(2));
    assert!(bus.failed_count() > 0);

    // Audit ledger records the failure without panicking
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.entries()[0].event_type, AuditEventType::CommandException);
}
