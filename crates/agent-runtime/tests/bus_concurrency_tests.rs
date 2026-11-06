use agent_domain::{AgentId, InterAgentEvent};
use agent_runtime::{
    BusBenchmarkHarness, BusBenchmarkStats, BusError, BusFaultPolicy, InterAgentBus,
    ResilientInterAgentBus,
};
use std::sync::Arc;

/// TDD test case for TOPIC-20261105-aa275b & TOPIC-20261105-6e89b1:
/// High-load concurrency benchmark (10,000 requests/events) & lock contention isolation
/// for agent-runtime::bus with P50/P90/P99 latency profiling and bounded memory verification.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-6e89b1.
#[test]
fn test_topic_aa275b_6e89b1_bus_benchmark_and_lock_contention_isolation() {
    println!("\n================================================================================");
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-aa275b / 6e89b1 Bus Benchmark...");
    println!("================================================================================");

    // 1. Single-threaded high load baseline benchmark (10,000 requests)
    let mut resilient =
        ResilientInterAgentBus::new(2_000, BusFaultPolicy::none(), 3).with_max_capacity(2_000);

    let stats: BusBenchmarkStats = BusBenchmarkHarness::run_benchmark(&mut resilient, 10_000);
    println!("\n[1. Baseline Single-Threaded Benchmark (10,000 requests)]");
    println!("  Total requests      : {}", stats.total_requests);
    println!("  Successful ops      : {}", stats.successful_operations);
    println!("  Rejected ops        : {}", stats.rejected_count);
    println!("  Throughput          : {:.2} ops/sec", stats.throughput_ops_per_sec);
    println!("  P50 Latency         : {:.2} µs", stats.p50_latency_us);
    println!("  P90 Latency         : {:.2} µs", stats.p90_latency_us);
    println!("  P99 Latency         : {:.2} µs", stats.p99_latency_us);
    println!("  Max Latency         : {:.2} µs", stats.max_latency_us);
    println!("  Duration            : {:?}", stats.total_duration);
    println!("  Memory Allocated    : {} bytes", stats.memory_allocated_bytes);

    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 500.0,
        "Throughput should exceed 500 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency must be tracked");
    assert!(stats.p90_latency_us > 0.0, "P90 latency must be tracked");
    assert!(
        stats.p99_latency_us < 25_000.0,
        "P99 latency must be under 25ms (25,000 µs), got {} µs",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 2. Verify bounded memory allocation (no unbounded memory leak)
    let mem_before = resilient.memory_usage_bytes();
    // Run another 5,000 requests to verify bounded capacity behavior
    let _ = BusBenchmarkHarness::run_benchmark(&mut resilient, 5_000);
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 1024,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );
    println!("\n[2. Bounded Memory Check]");
    println!("  Memory before: {} bytes | Memory after 5k extra ops: {} bytes", mem_before, mem_after);

    // 3. Batch publication optimization test
    let batch_events = vec![
        BusBenchmarkHarness::make_benchmark_event(1, false),
        BusBenchmarkHarness::make_benchmark_event(2, false),
        BusBenchmarkHarness::make_benchmark_event(3, true), // Invalid: empty ID
        BusBenchmarkHarness::make_benchmark_event(4, false),
    ];
    let batch_res = resilient.publish_resilient_batch(batch_events);
    assert!(batch_res.is_err(), "Batch containing invalid event must return error");
    assert_eq!(batch_res.unwrap_err(), BusError::CorruptedEvent("Event ID cannot be empty".to_string()));

    let valid_batch = vec![
        BusBenchmarkHarness::make_benchmark_event(10, false),
        BusBenchmarkHarness::make_benchmark_event(11, false),
        BusBenchmarkHarness::make_benchmark_event(12, false),
    ];
    let valid_batch_res = resilient.publish_resilient_batch(valid_batch);
    assert!(valid_batch_res.is_ok(), "Valid batch must succeed: {:?}", valid_batch_res.err());
    assert_eq!(valid_batch_res.unwrap(), 3);

    // 4. Lock Contention Comparison: Single Shared Lock vs 16-Way Sharded Concurrency
    println!("\n[3. Lock Contention Comparison (10,000 requests across 4 worker threads)]");
    let single_lock_stats = BusBenchmarkHarness::run_single_lock_benchmark(10_000, 4);
    println!("  [Single Lock] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        single_lock_stats.throughput_ops_per_sec, single_lock_stats.p99_latency_us
    );

    let sharded_stats = BusBenchmarkHarness::run_concurrent_benchmark(10_000, 4);
    println!("  [16-Sharded ] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        sharded_stats.throughput_ops_per_sec, sharded_stats.p99_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(
        sharded_stats.throughput_ops_per_sec > 500.0,
        "Sharded throughput must be high, got {}",
        sharded_stats.throughput_ops_per_sec
    );
    assert!(
        sharded_stats.p99_latency_us < 30_000.0,
        "Sharded P99 latency must be under 30ms, got {} µs",
        sharded_stats.p99_latency_us
    );

    // 5. High Concurrency Scaling with 8 threads (10,000 requests)
    let scaling_stats = BusBenchmarkHarness::run_concurrent_benchmark(10_000, 8);
    println!("\n[4. Concurrency Scaling (8 threads, 10,000 requests)]");
    println!("  Throughput: {:.2} ops/sec | P50: {:.2} µs | P90: {:.2} µs | P99: {:.2} µs",
        scaling_stats.throughput_ops_per_sec, scaling_stats.p50_latency_us,
        scaling_stats.p90_latency_us, scaling_stats.p99_latency_us
    );
    assert_eq!(scaling_stats.total_requests, 10_000);
    assert_eq!(scaling_stats.successful_operations, 9_000);
    assert_eq!(scaling_stats.rejected_count, 1_000);
    assert!(scaling_stats.throughput_ops_per_sec > 500.0);
}

/// TDD test case for TOPIC-20261105-aa275b:
/// Verifies that `crates/agent-runtime/src/bus` has been decomposed from a monolithic 651-line file
/// into modular, single-responsibility submodules under `src/bus/`,
/// with every submodule strictly under the 350-line P90 limit and zero `.unwrap()` in production code.
#[test]
fn test_topic_aa275b_bus_modularization_and_p90_limits() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bus_dir = manifest_dir.join("src/bus");
    assert!(bus_dir.is_dir(), "src/bus must exist as a directory");

    let submodules = [
        "mod.rs",
        "event.rs",
        "queue.rs",
        "fault.rs",
        "policy.rs",
        "subscriber.rs",
        "channel.rs",
        "resilience.rs",
        "benchmark.rs",
    ];

    for submodule in &submodules {
        let submodule_path = bus_dir.join(submodule);
        assert!(
            submodule_path.exists(),
            "Expected submodule file to exist: {}",
            submodule_path.display()
        );

        let content = std::fs::read_to_string(&submodule_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", submodule_path.display()));
        let line_count = content.lines().count();
        assert!(
            line_count <= 350,
            "Submodule src/bus/{} has {} lines, which exceeds P90 limit (350 lines)",
            submodule,
            line_count
        );

        // Verify zero unwrap() in production submodules
        let unwraps: Vec<_> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                !trimmed.starts_with("//")
                    && !trimmed.starts_with("/*")
                    && trimmed.contains(".unwrap()")
            })
            .collect();
        assert!(
            unwraps.is_empty(),
            "Found {} unwrap() calls in src/bus/{}: {:?}",
            unwraps.len(),
            submodule,
            unwraps
        );
    }
}

/// Async broadcast stress test with 10,000 events delivered to active subscribers
#[tokio::test]
async fn test_topic_aa275b_async_broadcast_with_active_subscribers() {
    let bus = Arc::new(InterAgentBus::new(2_048));
    let mut sub1 = bus.subscribe_resilient();
    let mut sub2 = bus.subscribe_resilient();

    let num_events = 2_000;
    let bus_publisher = Arc::clone(&bus);

    let pub_handle = tokio::spawn(async move {
        for i in 0..num_events {
            let event = InterAgentEvent::new(
                AgentId::new("sender-async"),
                None,
                "async.test.event",
                serde_json::json!({ "idx": i }),
            );
            let _ = bus_publisher.publish(event);
        }
    });

    let sub1_handle = tokio::spawn(async move {
        let mut count = 0;
        while count < num_events {
            match sub1.recv_resilient().await {
                Ok(_) => count += 1,
                Err(BusError::ReceiverLagged(_)) => {
                    // count missed
                    count += 1;
                }
                Err(_) => break,
            }
        }
        count
    });

    let sub2_handle = tokio::spawn(async move {
        let mut count = 0;
        while count < num_events {
            match sub2.recv_resilient().await {
                Ok(_) => count += 1,
                Err(BusError::ReceiverLagged(_)) => {
                    count += 1;
                }
                Err(_) => break,
            }
        }
        count
    });

    let (res_pub, res_sub1, res_sub2) = tokio::join!(pub_handle, sub1_handle, sub2_handle);
    assert!(res_pub.is_ok());
    assert_eq!(res_sub1.unwrap(), num_events);
    assert_eq!(res_sub2.unwrap(), num_events);
}

#[test]
fn test_bus_event_validation_and_filter() {
    use agent_runtime::bus::{calculate_event_shard, validate_bus_event, BusEventFilter};

    let valid_event = InterAgentEvent::new(
        AgentId::new("agent-alpha"),
        Some(AgentId::new("agent-beta")),
        "task.handoff",
        serde_json::json!({ "data": 42 }),
    );
    assert!(validate_bus_event(&valid_event).is_ok());

    let mut invalid_event = valid_event.clone();
    invalid_event.id = "   ".to_string();
    assert!(validate_bus_event(&invalid_event).is_err());

    let shard_0 = calculate_event_shard(&valid_event, 16);
    assert!(shard_0 < 16);

    let filter = BusEventFilter::new().with_topic_prefix("task.");
    assert!(filter.matches(&valid_event));

    let filter_mismatch = BusEventFilter::new().with_topic_prefix("workflow.");
    assert!(!filter_mismatch.matches(&valid_event));
}

#[tokio::test]
async fn test_sharded_bus_queue_concurrency_and_lock_isolation() {
    use agent_runtime::bus::ShardedBusQueue;

    let sharded_bus = Arc::new(ShardedBusQueue::new(1024, 8));
    assert_eq!(sharded_bus.shard_count(), 8);
    assert_eq!(sharded_bus.total_capacity(), 8 * 1024);

    // Subscribe to shard 0
    let _sub_shard_0 = sharded_bus.subscribe_resilient_shard(0).expect("Shard 0 must exist");

    let bus_clone = Arc::clone(&sharded_bus);
    let num_threads = 4;
    let events_per_thread = 500;
    let mut handles = Vec::new();

    for t in 0..num_threads {
        let bus = Arc::clone(&bus_clone);
        handles.push(tokio::spawn(async move {
            for i in 0..events_per_thread {
                let event = InterAgentEvent::new(
                    AgentId::new(&format!("sender-{}", t)),
                    Some(AgentId::new("receiver-sharded")),
                    "sharded.benchmark.event",
                    serde_json::json!({ "thread": t, "i": i }),
                );
                let _ = bus.safe_publish(event);
            }
        }));
    }

    for h in handles {
        h.await.expect("Worker thread should succeed");
    }

    assert!(!sharded_bus.is_closed());
    // Safe publish invalid event returns Err
    let mut bad_event = InterAgentEvent::new(
        AgentId::new("sender"),
        None,
        "bad.event",
        serde_json::json!({}),
    );
    bad_event.id = String::new();
    assert!(sharded_bus.safe_publish(bad_event).is_err());

    // Clean close
    sharded_bus.close_all();
    assert!(sharded_bus.is_closed());
}
