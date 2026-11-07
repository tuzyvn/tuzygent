use agent_engine::EngineInput;
use agent_runtime::{
    ExecutionBenchmarkHarness, ExecutionBenchmarkStats, ExecutionFaultPolicy, ExecutionService,
    ResilientExecutionService,
};
use std::sync::Arc;

/// TDD test case for TOPIC-20261105-884ce9 & TOPIC-20261105-04c010:
/// High-load concurrency benchmark (10,000 requests) & lock contention isolation
/// for agent-runtime::execution with P50/P90/P99 latency profiling and bounded memory verification.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-04c010.
#[tokio::test]
async fn test_topic_884ce9_04c010_execution_benchmark_and_lock_contention_isolation() {
    println!("\n================================================================================");
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-884ce9 / 04c010 Execution Benchmark...");
    println!("================================================================================");

    // 1. Single-threaded high load baseline benchmark (10,000 requests)
    let service = ExecutionService::new("/tmp", None, None);
    let mut resilient = ResilientExecutionService::new(service, ExecutionFaultPolicy::none(), 3)
        .with_max_capacity(2_000);

    let stats: ExecutionBenchmarkStats =
        ExecutionBenchmarkHarness::run_benchmark(&mut resilient, 10_000).await;
    println!("\n[1. Baseline Single-Threaded Benchmark (10,000 requests)]");
    println!("  Total requests      : {}", stats.total_requests);
    println!("  Successful ops      : {}", stats.successful_operations);
    println!("  Rejected ops        : {}", stats.rejected_count);
    println!(
        "  Throughput          : {:.2} ops/sec",
        stats.throughput_ops_per_sec
    );
    println!("  P50 Latency         : {:.2} µs", stats.p50_latency_us);
    println!("  P90 Latency         : {:.2} µs", stats.p90_latency_us);
    println!("  P99 Latency         : {:.2} µs", stats.p99_latency_us);
    println!("  Max Latency         : {:.2} µs", stats.max_latency_us);
    println!("  Duration            : {:?}", stats.total_duration);
    println!(
        "  Memory Allocated    : {} bytes",
        stats.memory_allocated_bytes
    );

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
        stats.p99_latency_us < 50_000.0,
        "P99 latency must be under 50ms (50,000 µs), got {} µs",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 2. Verify bounded memory allocation (no unbounded memory leak / FIFO eviction)
    let mem_before = resilient.memory_usage_bytes();
    let _ = ExecutionBenchmarkHarness::run_benchmark(&mut resilient, 5_000).await;
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );
    println!("\n[2. Bounded Memory Check]");
    println!(
        "  Memory before: {} bytes | Memory after 5k extra ops: {} bytes",
        mem_before, mem_after
    );

    // 3. Batch execution optimization test
    let mock_engine = Arc::new(agent_runtime::execution::benchmark::BenchmarkMockEngine);
    let batch_inputs = vec![
        ExecutionBenchmarkHarness::make_benchmark_input(1, false),
        ExecutionBenchmarkHarness::make_benchmark_input(2, false),
        ExecutionBenchmarkHarness::make_benchmark_input(3, true), // Prohibited command pattern
        ExecutionBenchmarkHarness::make_benchmark_input(4, false),
    ];
    let batch_res = resilient.execute_batch(mock_engine, &batch_inputs).await;
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(batch_res[2].is_err(), "Invalid command must fail in batch");
    assert!(batch_res[3].is_ok());

    // 4. Lock Contention Comparison: Single Shared Lock vs 16-Way Sharded Concurrency
    println!("\n[3. Lock Contention Comparison (10,000 requests across 4 worker threads)]");
    let single_lock_stats = ExecutionBenchmarkHarness::run_single_lock_benchmark(10_000, 4).await;
    println!(
        "  [Single Lock] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        single_lock_stats.throughput_ops_per_sec, single_lock_stats.p99_latency_us
    );

    let sharded_stats = ExecutionBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;
    println!(
        "  [16-Sharded ] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        sharded_stats.throughput_ops_per_sec, sharded_stats.p99_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(
        sharded_stats.throughput_ops_per_sec > 200.0,
        "Sharded throughput must be high, got {}",
        sharded_stats.throughput_ops_per_sec
    );

    // 5. High Concurrency Scaling with 8 tasks (10,000 requests)
    let scaling_stats = ExecutionBenchmarkHarness::run_concurrent_benchmark(10_000, 8).await;
    println!("\n[4. Concurrency Scaling (8 tasks, 10,000 requests)]");
    println!(
        "  Throughput: {:.2} ops/sec | P50: {:.2} µs | P90: {:.2} µs | P99: {:.2} µs",
        scaling_stats.throughput_ops_per_sec,
        scaling_stats.p50_latency_us,
        scaling_stats.p90_latency_us,
        scaling_stats.p99_latency_us
    );
    assert_eq!(scaling_stats.total_requests, 10_000);
    assert_eq!(scaling_stats.successful_operations, 9_000);
    assert_eq!(scaling_stats.rejected_count, 1_000);
    assert!(scaling_stats.throughput_ops_per_sec > 200.0);
}

/// TDD test case verifying modular architecture, line limits (<= 350 lines),
/// and zero unwrap in agent-runtime::execution production code.
#[test]
fn test_topic_884ce9_execution_modularization_and_p90_limits() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let exec_file = manifest_dir.join("src/execution.rs");
    assert!(exec_file.is_file(), "src/execution.rs must exist");

    let exec_content = std::fs::read_to_string(&exec_file)
        .unwrap_or_else(|_| panic!("Failed to read {}", exec_file.display()));
    let exec_lines = exec_content.lines().count();
    assert!(
        exec_lines <= 350,
        "src/execution.rs has {} lines, which exceeds P90 limit (350 lines)",
        exec_lines
    );

    let submodules = ["fault.rs", "resilience.rs", "benchmark.rs"];
    let exec_dir = manifest_dir.join("src/execution");
    for sub in &submodules {
        let path = exec_dir.join(sub);
        assert!(path.is_file(), "Expected {} to exist", path.display());
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read {}", path.display()));
        let lines = content.lines().count();
        assert!(
            lines <= 350,
            "src/execution/{} has {} lines, which exceeds P90 limit (350 lines)",
            sub,
            lines
        );

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
            "Found {} unwrap() calls in src/execution/{}: {:?}",
            unwraps.len(),
            sub,
            unwraps
        );
    }
}

#[test]
fn test_execution_input_and_benchmark_stats_creation() {
    let inp_valid = ExecutionBenchmarkHarness::make_benchmark_input(42, false);
    assert!(inp_valid.prompt_or_command.contains("42"));
    assert!(!inp_valid.prompt_or_command.contains("rm -rf"));

    let inp_invalid = ExecutionBenchmarkHarness::make_benchmark_input(99, true);
    assert!(inp_invalid.prompt_or_command.contains("rm -rf"));

    let empty_input = EngineInput::new("", "/tmp");
    assert!(empty_input.prompt_or_command.is_empty());
}
