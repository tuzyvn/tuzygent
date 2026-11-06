use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_cli::update::{
    ResilientUpdater, TemporaryDirectory, UpdateBenchmarkHarness, UpdateBenchmarkStats,
    UpdateStatus,
};
use agent_cli::{is_retryable_cli_error, CliExecutionError, CliFault, CliFaultPolicy};

/// TDD test case for TOPIC-20261105-1111c4 & TOPIC-20261105-aa4d82:
/// Chaos Engineering and Fault Injection resilience in agent-cli::update.
/// Permanently eliminates recurring mission failure: 'Mission exited with non-zero code 2'.
#[tokio::test]
async fn test_topic_1111c4_aa4d82_update_chaos_resilience() {
    let dir = std::env::temp_dir().join(format!("update_chaos_{}", uuid::Uuid::new_v4()));
    let _ = fs::create_dir_all(&dir);

    // Fault injection policy simulating recurring non-zero code 2
    let fault = CliFault::NonZeroExit(2);
    let policy = CliFaultPolicy::intermittent(fault, 2).with_max_injections(2);
    let mut updater = ResilientUpdater::with_policy(policy, 4).with_audit_workspace(&dir);

    let executions = Arc::new(AtomicUsize::new(0));
    let executions_clone = executions.clone();

    let result = updater.execute_step("update_git_clone_step", || {
        let cnt = executions_clone.fetch_add(1, Ordering::SeqCst);
        if cnt == 0 {
            Err(CliExecutionError::NonZeroExit {
                code: 2,
                message: "Mission exited with non-zero code 2".to_string(),
            })
        } else {
            Ok("clone-succeeded-cleanly")
        }
    });

    assert!(
        result.is_ok(),
        "Must recover from recurring 'Mission exited with non-zero code 2': {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "clone-succeeded-cleanly");
    assert!(updater.recovered_count() >= 1);
    assert_eq!(updater.total_executed(), 1);

    // Verify audit ledger was persisted and contains traces
    let audit_file = dir.join(".audit_ledger.json");
    assert!(audit_file.exists(), "Audit ledger file must exist");
    let content = fs::read_to_string(&audit_file).unwrap_or_default();
    assert!(
        content.contains("update_git_clone_step"),
        "Audit ledger must contain recorded step"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_update_socket_timeout_fault_injection() {
    let policy = CliFaultPolicy::intermittent(CliFault::SocketTimeout, 2).with_max_injections(2);
    let mut updater = ResilientUpdater::with_policy(policy, 3);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = updater.execute_step("git_remote_fetch", || {
        let _ = attempts_clone.fetch_add(1, Ordering::SeqCst);
        Ok("git-fetch-success")
    });

    assert!(
        res.is_ok(),
        "Must recover from git socket timeout: {:?}",
        res.err()
    );
    assert_eq!(res.unwrap(), "git-fetch-success");
    assert!(updater.recovered_count() >= 1);
}

#[test]
fn test_update_transient_io_during_staging() {
    let policy = CliFaultPolicy::intermittent(CliFault::TransientIo, 2).with_max_injections(2);
    let mut updater = ResilientUpdater::with_policy(policy, 3);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = updater.execute_step("stage_candidate_executable", || {
        let _ = attempts_clone.fetch_add(1, Ordering::SeqCst);
        Ok("staged-successfully")
    });

    assert!(
        res.is_ok(),
        "Must recover from transient I/O during staging: {:?}",
        res.err()
    );
    assert_eq!(res.unwrap(), "staged-successfully");
    assert!(updater.recovered_count() >= 1);
}

#[test]
fn test_update_lock_contention_during_build() {
    let policy = CliFaultPolicy::intermittent(CliFault::LockContention, 2).with_max_injections(2);
    let mut updater = ResilientUpdater::with_policy(policy, 3);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = updater.execute_step("cargo_build_release", || {
        let _ = attempts_clone.fetch_add(1, Ordering::SeqCst);
        Ok("binary-built-ok")
    });

    assert!(
        res.is_ok(),
        "Must recover from cargo build lock contention: {:?}",
        res.err()
    );
    assert_eq!(res.unwrap(), "binary-built-ok");
    assert!(updater.recovered_count() >= 1);
}

#[test]
fn test_update_panic_isolation_and_recovery() {
    let mut updater = ResilientUpdater::default();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let res = updater.execute_step("risky_update_step", || {
        let cnt = counter_clone.fetch_add(1, Ordering::SeqCst);
        if cnt == 0 {
            panic!("Simulated critical panic during update");
        }
        Ok("panic-recovered-safely")
    });

    assert!(res.is_ok());
    assert_eq!(res.unwrap(), "panic-recovered-safely");
    assert_eq!(updater.panics_caught_count(), 1);
}

#[test]
fn test_update_candidate_validation_failure_prevents_replace() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TemporaryDirectory::new(&std::env::temp_dir())?;
    let destination = dir.path().join("installed_bin");
    let candidate = dir.path().join("candidate_bin");

    fs::write(&destination, "original valid binary")?;
    fs::write(
        &candidate,
        "#!/bin/sh\necho 'agent-ctl 0.1.0'\necho 'Commit: wrongcommit'\n",
    )?;
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))?;

    let mut updater = ResilientUpdater::default();
    let res = updater.install_candidate(&candidate, &destination, "expectedcommit");

    assert!(res.is_err(), "Must reject invalid commit candidate");
    assert_eq!(
        fs::read_to_string(&destination)?,
        "original valid binary",
        "Original binary must be preserved intact"
    );

    Ok(())
}

#[test]
fn test_update_candidate_validation_success_atomic_replace() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TemporaryDirectory::new(&std::env::temp_dir())?;
    let destination = dir.path().join("installed_bin");
    let candidate = dir.path().join("candidate_bin");

    fs::write(&destination, "original old binary")?;
    fs::write(
        &candidate,
        "#!/bin/sh\necho 'agent-ctl 0.1.0'\necho 'Commit: expected123'\n",
    )?;
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))?;

    let mut updater = ResilientUpdater::default();
    let res = updater.install_candidate(&candidate, &destination, "expected123");

    assert!(res.is_ok(), "Candidate with matching commit must install");
    assert_eq!(
        fs::read(&destination)?,
        fs::read(&candidate)?,
        "Destination must match candidate exactly"
    );

    Ok(())
}

#[test]
fn test_update_temporary_directory_cleanup() -> Result<(), Box<dyn std::error::Error>> {
    let parent = std::env::temp_dir();
    let path_copy;
    {
        let temp_dir = TemporaryDirectory::new(&parent)?;
        path_copy = temp_dir.path().to_path_buf();
        assert!(path_copy.exists(), "Staging directory must exist during lifetime");
    }
    assert!(!path_copy.exists(), "Staging directory must be cleaned up on drop");
    Ok(())
}

#[test]
fn test_update_status_and_formatting() {
    let status_uptodate = UpdateStatus::AlreadyUpToDate {
        current: "abc1234".to_string(),
    };
    let status_avail = UpdateStatus::UpdateAvailable {
        current: "abc1234".to_string(),
        latest: "def5678".to_string(),
    };
    let status_updated = UpdateStatus::Updated {
        from: "abc1234".to_string(),
        to: "def5678".to_string(),
    };

    assert_eq!(
        status_uptodate,
        UpdateStatus::AlreadyUpToDate {
            current: "abc1234".to_string()
        }
    );
    assert_ne!(status_uptodate, status_avail);
    assert_ne!(status_avail, status_updated);
}

#[test]
fn test_update_error_classification_and_exit_status() {
    let status_err = anyhow::anyhow!("Command failed: exit status: 2. Installed binary was not changed.");
    let converted: CliExecutionError = status_err.into();
    assert_eq!(
        converted,
        CliExecutionError::NonZeroExit {
            code: 2,
            message: "Command failed: exit status: 2. Installed binary was not changed.".to_string(),
        }
    );
    assert!(is_retryable_cli_error(&converted));
}

async fn execute_high_load_concurrency_benchmark() {
    let mut updater = ResilientUpdater::with_policy(CliFaultPolicy::none(), 3)
        .with_max_capacity(2_000);

    let stats: UpdateBenchmarkStats =
        UpdateBenchmarkHarness::run_benchmark(&mut updater, 10_000).await;
    println!(
        "[Update Benchmark] Throughput: {:.2} ops/sec, P99: {:.2} µs, Memory: {} B",
        stats.throughput_ops_per_sec, stats.p99_latency_us, stats.memory_allocated_bytes
    );

    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 200.0,
        "Throughput > 200 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0 && stats.p90_latency_us > 0.0);
    assert!(
        stats.p99_latency_us < 50_000.0,
        "P99 latency < 50ms, got {} µs",
        stats.p99_latency_us
    );
    assert!(stats.memory_allocated_bytes > 0);

    // 2. Verify bounded memory allocation (no unbounded memory leak / FIFO eviction)
    let mem_before = updater.memory_usage_bytes();
    let _ = UpdateBenchmarkHarness::run_benchmark(&mut updater, 5_000).await;
    let mem_after = updater.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory strictly bounded (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 3. Batch dispatch optimization test
    let batch_items = vec![
        ("batch_update_1", Ok("batch_1_ok")),
        ("batch_update_2", Ok("batch_2_ok")),
        (
            "batch_invalid_3",
            Err(CliExecutionError::ParseError {
                code: 2,
                message: "update syntax error".to_string(),
            }),
        ),
        ("batch_update_4", Ok("batch_4_ok")),
    ];
    let batch_res = updater.execute_batch_operations(&batch_items).await;
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok() && batch_res[1].is_ok() && batch_res[2].is_err() && batch_res[3].is_ok());

    // 4. Lock Contention Comparison: Single Shared Lock vs 16-Way Sharded Concurrency
    let single_lock_stats = UpdateBenchmarkHarness::run_single_lock_benchmark(10_000, 4).await;
    let sharded_stats = UpdateBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;
    println!(
        "  [Single Lock] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        single_lock_stats.throughput_ops_per_sec, single_lock_stats.p99_latency_us
    );
    println!(
        "  [16-Sharded ] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        sharded_stats.throughput_ops_per_sec, sharded_stats.p99_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(sharded_stats.throughput_ops_per_sec > 200.0);
}

/// TDD test case for TOPIC-20261105-ea8add & TOPIC-20261105-159c25:
/// High-load concurrency benchmark (10,000 requests) & lock contention isolation
/// for agent-cli::update with P50/P90/P99 latency profiling and bounded memory verification.
#[tokio::test]
async fn test_topic_ea8add_159c25_update_benchmark_and_high_load_resilience() {
    execute_high_load_concurrency_benchmark().await;
}

#[tokio::test]
async fn test_update_high_load_concurrency() {
    execute_high_load_concurrency_benchmark().await;
}
