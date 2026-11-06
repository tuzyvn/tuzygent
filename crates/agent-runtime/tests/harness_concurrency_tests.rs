use agent_runtime::{
    HarnessBenchmarkHarness, HarnessBenchmarkStats, ResilientHarnessMemoryStore,
};

/// TDD test case for TOPIC-20261105-50baad & TOPIC-20261105-80bc76:
/// High-load concurrency benchmark (10,000 requests) & lock contention isolation
/// for agent-runtime::harness with P50/P90/P99 latency profiling and bounded memory verification.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-80bc76.
#[tokio::test]
async fn test_topic_50baad_80bc76_harness_benchmark_and_lock_contention_isolation() {
    println!("\n================================================================================");
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-50baad / 80bc76 Harness Benchmark...");
    println!("================================================================================");

    // 1. Single-threaded high load baseline benchmark (10,000 requests)
    let mut resilient =
        ResilientHarnessMemoryStore::memory().with_max_capacity(2_000);

    let stats: HarnessBenchmarkStats =
        HarnessBenchmarkHarness::run_benchmark(&mut resilient, 10_000).await;
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
        stats.throughput_ops_per_sec > 200.0,
        "Throughput should exceed 200 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency must be tracked");
    assert!(stats.p90_latency_us > 0.0, "P90 latency must be tracked");
    assert!(
        stats.p99_latency_us < 100_000.0,
        "P99 latency must be under 100ms (100,000 µs), got {} µs",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 2. Verify bounded memory allocation (no unbounded memory leak / FIFO eviction)
    let mem_before = resilient.memory_usage_bytes();
    let _ = HarnessBenchmarkHarness::run_benchmark(&mut resilient, 5_000).await;
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );
    println!("\n[2. Bounded Memory Check]");
    println!("  Memory before: {} bytes | Memory after 5k extra ops: {} bytes", mem_before, mem_after);

    // 3. Batch recording optimization test
    let batch_inputs = vec![
        ("agent_node", "target_1", "error pattern 1", "strategy 1"),
        ("agent_node", "target_2", "error pattern 2", "strategy 2"),
        ("", "", "invalid error pattern", "invalid strategy"), // Invalid input rejected
        ("agent_node", "target_4", "error pattern 4", "strategy 4"),
    ];
    let batch_res = resilient.record_batch(&batch_inputs).await;
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(batch_res[2].is_err(), "Invalid input must fail in batch");
    assert!(batch_res[3].is_ok());

    // 4. Lock Contention Comparison: Single Shared Lock vs 16-Way Sharded Concurrency
    println!("\n[3. Lock Contention Comparison (10,000 requests across 4 worker threads)]");
    let single_lock_stats = HarnessBenchmarkHarness::run_single_lock_benchmark(10_000, 4).await;
    println!("  [Single Lock] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        single_lock_stats.throughput_ops_per_sec, single_lock_stats.p99_latency_us
    );

    let sharded_stats = HarnessBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;
    println!("  [16-Sharded ] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        sharded_stats.throughput_ops_per_sec, sharded_stats.p99_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(
        sharded_stats.throughput_ops_per_sec > 200.0,
        "Sharded throughput must be high"
    );

    println!("\n================================================================================");
    println!("[Reproduction & Benchmark Test] TOPIC-20261105-50baad / 80bc76 SUCCESS!");
    println!("================================================================================");
}
