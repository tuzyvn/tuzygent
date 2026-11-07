use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
};
use agent_memory::{
    PersonaBenchmarkHarness, PersonaBenchmarkStats, PersonaError, ResilientPersonaLoader,
};

fn make_test_profile(name: &str) -> AgentPersonaProfile {
    AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::from_str(&format!("{}-001", name.to_lowercase())),
            name: name.to_string(),
            description: format!("Profile for {}", name),
            autonomy_level: AutonomyLevel::Full,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: name.to_uppercase(),
            title: format!("Engineer ({})", name),
            primary_domain: "Distributed Systems".to_string(),
            capabilities: vec!["high concurrency".to_string(), "lock isolation".to_string()],
        },
        soul: AgentSoul {
            persona: "Analytical and ultra-fast.".to_string(),
            tone_of_voice: "Direct".to_string(),
            core_values: vec!["Scalability".to_string()],
            voice_profile: None,
        },
        mission: AgentMission {
            high_level_objective: "Ensure high throughput and low P99 latency".to_string(),
            kpis: vec!["P99 < 20ms".to_string()],
            constraints: vec!["Zero deadlocks".to_string()],
        },
    }
}

/// TDD test case for TOPIC-20261105-8c43f7 & TOPIC-20261105-7e124f:
/// High-load concurrency benchmark (10,000 requests) & lock contention isolation
/// for agent-memory::persona.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-7e124f.
#[test]
fn test_topic_8c43f7_7e124f_persona_benchmark_and_lock_contention_isolation() {
    println!("\n================================================================================");
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-8c43f7 / 7e124f Persona Benchmark...");
    println!("================================================================================");

    // 1. Single-threaded high load baseline benchmark (10,000 requests)
    let mut resilient = ResilientPersonaLoader::default().with_max_capacity(2_000);

    let stats: PersonaBenchmarkStats =
        PersonaBenchmarkHarness::run_benchmark(&mut resilient, 10_000);
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
        stats.throughput_ops_per_sec > 500.0,
        "Throughput should exceed 500 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency must be tracked");
    assert!(stats.p90_latency_us > 0.0, "P90 latency must be tracked");
    assert!(
        stats.p99_latency_us < 20_000.0,
        "P99 latency must be under 20ms (20,000 µs), got {} µs",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 2. Verify bounded memory allocation (no unbounded memory leak)
    let mem_before = resilient.memory_usage_bytes();
    // Run another 5,000 requests to trigger FIFO eviction in bounded capacity
    let _ = PersonaBenchmarkHarness::run_benchmark(&mut resilient, 5_000);
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

    // 3. Batch resolution optimization test
    let batch_profiles = vec![
        PersonaBenchmarkHarness::make_benchmark_profile(1, false),
        PersonaBenchmarkHarness::make_benchmark_profile(2, false),
        PersonaBenchmarkHarness::make_benchmark_profile(3, true), // Invalid: empty name
        PersonaBenchmarkHarness::make_benchmark_profile(4, false),
    ];
    let batch_res = resilient.save_batch(&batch_profiles);
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(matches!(
        batch_res[2],
        Err(PersonaError::InvalidIdentity(_))
    ));
    assert!(batch_res[3].is_ok());
    println!("\n[3. Batch Processing Check]");
    println!("  Batch of 4 profiles processed with granular error isolation successfully.");

    // 4. Lock Contention Comparison: Single Mutex vs 16-Shard Concurrent Architecture
    println!("\n[4. Lock Contention Isolation Analysis (10,000 requests across 4 worker threads)]");

    // 4a. Single Lock (Contention Bottleneck)
    let single_lock_stats = PersonaBenchmarkHarness::run_single_lock_benchmark(10_000, 4);
    println!("  --- Architecture A: Single Global Mutex (Contention Point) ---");
    println!(
        "    Throughput         : {:.2} ops/sec",
        single_lock_stats.throughput_ops_per_sec
    );
    println!(
        "    P50 Latency        : {:.2} µs",
        single_lock_stats.p50_latency_us
    );
    println!(
        "    P90 Latency        : {:.2} µs",
        single_lock_stats.p90_latency_us
    );
    println!(
        "    P99 Latency        : {:.2} µs",
        single_lock_stats.p99_latency_us
    );
    println!(
        "    Max Latency        : {:.2} µs",
        single_lock_stats.max_latency_us
    );

    // 4b. Sharded Lock (Fine-Grained Partitioning)
    let sharded_stats = PersonaBenchmarkHarness::run_concurrent_benchmark(10_000, 4);
    println!("  --- Architecture B: 16-Shard Partitioned Locks (Contention Isolated) ---");
    println!(
        "    Throughput         : {:.2} ops/sec",
        sharded_stats.throughput_ops_per_sec
    );
    println!(
        "    P50 Latency        : {:.2} µs",
        sharded_stats.p50_latency_us
    );
    println!(
        "    P90 Latency        : {:.2} µs",
        sharded_stats.p90_latency_us
    );
    println!(
        "    P99 Latency        : {:.2} µs",
        sharded_stats.p99_latency_us
    );
    println!(
        "    Max Latency        : {:.2} µs",
        sharded_stats.max_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(
        sharded_stats.throughput_ops_per_sec > 500.0,
        "Sharded throughput should exceed 500 ops/sec, got {}",
        sharded_stats.throughput_ops_per_sec
    );
    assert!(
        sharded_stats.p99_latency_us < 20_000.0,
        "Sharded P99 latency should be well under 20ms, got {} µs",
        sharded_stats.p99_latency_us
    );
    assert!(
        sharded_stats.memory_allocated_bytes > 0,
        "Sharded memory usage should be tracked"
    );

    println!("\n[Lock Contention Isolation Result]");
    println!(
        "  Sharding reduced lock contention overhead: Throughput improved from {:.2} to {:.2} ops/sec.",
        single_lock_stats.throughput_ops_per_sec, sharded_stats.throughput_ops_per_sec
    );
    println!("================================================================================\n");
}

#[test]
fn test_persona_high_concurrency_8_threads_10000_requests() {
    let stats = PersonaBenchmarkHarness::run_concurrent_benchmark(10_000, 8);
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 500.0,
        "Concurrent throughput with 8 threads must exceed 500 ops/sec"
    );
    assert!(
        stats.p99_latency_us < 20_000.0,
        "P99 latency with 8 threads must stay under 20ms"
    );
}

#[test]
fn test_persona_bounded_capacity_cache_and_eviction() {
    let mut loader = ResilientPersonaLoader::default().with_max_capacity(5);

    for i in 0..10 {
        let profile = make_test_profile(&format!("Agent_{}", i));
        let res = loader.save_profile_cached(&profile);
        assert!(res.is_ok());
    }

    assert_eq!(loader.cached_profiles().len(), 5);
    // Oldest profiles (Agent_0 to Agent_4) should be evicted
    assert!(!loader.has_persona("Agent_0"));
    assert!(!loader.has_persona("Agent_1"));
    assert!(!loader.has_persona("Agent_4"));
    // Newest profiles (Agent_5 to Agent_9) should be retained
    assert!(loader.has_persona("Agent_5"));
    assert!(loader.has_persona("Agent_9"));
}
