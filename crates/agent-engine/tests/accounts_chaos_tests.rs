use std::collections::HashMap;
use std::sync::Arc;
use agent_domain::{ActionSeverity, AuditLedger};
use agent_engine::{
    AccountBenchmarkHarness, AccountEngineKind, AccountError, AccountFault, AccountFaultPolicy,
    AccountProfile, AccountStatus, AccountVault, ResilientAccountManager,
};

fn temp_vault_dir(prefix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vault_chaos_{}_{}",
        prefix,
        uuid::Uuid::new_v4().simple()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn create_sample_profile(id: &str, name: &str, engine: AccountEngineKind) -> AccountProfile {
    let home = std::env::temp_dir().join("accounts_test_home").join(id);
    AccountProfile {
        id: id.to_string(),
        name: name.to_string(),
        engine,
        home_dir: home,
        status: AccountStatus::Ready,
        requests_count: 0,
        failure_count: 0,
        last_used_at: None,
        extra_env: HashMap::new(),
    }
}

#[test]
fn test_account_validation_and_integrity() {
    let valid_profile = create_sample_profile("acc_1", "Primary Account", AccountEngineKind::Antigravity);
    assert!(valid_profile.validate().is_ok());

    // Profile with empty name must fail validation
    let mut invalid_name_profile = valid_profile.clone();
    invalid_name_profile.name = "   ".to_string();
    assert!(matches!(
        invalid_name_profile.validate(),
        Err(AccountError::InvalidAccount(_))
    ));

    // Profile with empty ID must fail validation
    let mut invalid_id_profile = valid_profile.clone();
    invalid_id_profile.id = "".to_string();
    assert!(matches!(
        invalid_id_profile.validate(),
        Err(AccountError::InvalidAccount(_))
    ));

    // Profile with path traversal ID must fail validation
    let mut traversal_profile = valid_profile.clone();
    traversal_profile.id = "../etc/passwd".to_string();
    assert!(matches!(
        traversal_profile.validate(),
        Err(AccountError::InvalidAccount(_))
    ));

    let mut traversal_slash = valid_profile.clone();
    traversal_slash.id = "bad/account".to_string();
    assert!(matches!(
        traversal_slash.validate(),
        Err(AccountError::InvalidAccount(_))
    ));

    // Verify simulate_fault on AccountProfile
    assert!(matches!(
        valid_profile.simulate_fault(&AccountFault::SocketTimeout),
        Err(AccountError::SocketTimeout)
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&AccountFault::ConnectionReset),
        Err(AccountError::ConnectionReset)
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&AccountFault::RateLimitExceeded),
        Err(AccountError::RateLimitExceeded(_))
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&AccountFault::TransientIo),
        Err(AccountError::TransientIo(_))
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&AccountFault::CorruptedMetadata),
        Err(AccountError::CorruptedMetadata(_))
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&AccountFault::ServiceUnavailable),
        Err(AccountError::ServiceUnavailable(_))
    ));
}

#[test]
fn test_account_intermittent_socket_timeout_recovery() {
    let temp_dir = temp_vault_dir("socket_timeout");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_main", "Main Account", HashMap::new())
        .expect("registration should succeed");

    // Intermittent socket timeout every 2 invocations, max retries 3
    let fault_policy = AccountFaultPolicy::intermittent(AccountFault::SocketTimeout, 2);
    let mut manager = ResilientAccountManager::new(vault, fault_policy, 3);

    let res = manager.select_account(&AccountEngineKind::Antigravity);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_max_retries_exceeded_on_persistent_timeout() {
    let temp_dir = temp_vault_dir("persistent_timeout");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Codex, "acc_codex", "Codex Account", HashMap::new())
        .expect("registration should succeed");

    // Always inject SocketTimeout, max retries 2
    let fault_policy = AccountFaultPolicy::always(AccountFault::SocketTimeout);
    let mut manager = ResilientAccountManager::new(vault, fault_policy, 2);

    let res = manager.select_account(&AccountEngineKind::Codex);
    assert_eq!(res, Err(AccountError::SocketTimeout));
    assert_eq!(manager.resolved_count(), 0);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_connection_reset_recovery() {
    let temp_dir = temp_vault_dir("conn_reset");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Claude, "acc_claude", "Claude Account", HashMap::new())
        .expect("registration should succeed");

    let fault_policy = AccountFaultPolicy::intermittent(AccountFault::ConnectionReset, 2);
    let mut manager = ResilientAccountManager::new(vault, fault_policy, 3);

    let res = manager.select_account(&AccountEngineKind::Claude);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_transient_io_failure_recovery() {
    let temp_dir = temp_vault_dir("transient_io");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_io", "IO Account", HashMap::new())
        .expect("registration should succeed");

    let fault_policy = AccountFaultPolicy::intermittent(AccountFault::TransientIo, 2);
    let mut manager = ResilientAccountManager::new(vault, fault_policy, 3);

    let res = manager.select_account(&AccountEngineKind::Antigravity);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_corrupted_metadata_rejection() {
    let temp_dir = temp_vault_dir("corrupted_meta");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_corrupt", "Corrupt Account", HashMap::new())
        .expect("registration should succeed");

    let fault_policy = AccountFaultPolicy::always(AccountFault::CorruptedMetadata);
    let mut manager = ResilientAccountManager::new(vault, fault_policy, 3);

    let res = manager.select_account(&AccountEngineKind::Antigravity);
    assert!(matches!(res, Err(AccountError::CorruptedMetadata(_))));
    assert_eq!(manager.resolved_count(), 0);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_service_unavailable_recovery() {
    let temp_dir = temp_vault_dir("service_unavail");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_service", "Service Account", HashMap::new())
        .expect("registration should succeed");

    let fault_policy = AccountFaultPolicy::intermittent(AccountFault::ServiceUnavailable, 2);
    let mut manager = ResilientAccountManager::new(vault, fault_policy, 3);

    let res = manager.select_account(&AccountEngineKind::Antigravity);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_rate_limit_rotation_and_cooldown_failover() {
    let temp_dir = temp_vault_dir("rate_limit_rotation");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));

    // Đăng ký 2 tài khoản cho cùng một engine
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_alpha", "Alpha Account", HashMap::new())
        .expect("register alpha");
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_beta", "Beta Account", HashMap::new())
        .expect("register beta");

    // Tiêm lỗi RateLimit ngắt quãng ở lần thử đầu tiên (max_injections = 1)
    let fault_policy = AccountFaultPolicy::intermittent(AccountFault::RateLimitExceeded, 1)
        .with_max_injections(1);
    let mut manager = ResilientAccountManager::new(vault.clone(), fault_policy, 3);

    // Lần chọn đầu tiên: acc_alpha gặp rate limit -> tự động đưa vào cooldown -> failover bốc acc_beta thành công
    let selected = manager.select_account(&AccountEngineKind::Antigravity).expect("failover should succeed");
    assert_eq!(selected.id, "acc_beta");
    assert_eq!(manager.cooldown_count(), 1);
    assert_eq!(manager.recovered_count(), 1);
    assert_eq!(manager.resolved_count(), 1);

    // Xác nhận acc_alpha đang ở trạng thái Cooldown trong vault
    let alpha = vault.get_account(&AccountEngineKind::Antigravity, "acc_alpha").expect("alpha exists");
    assert!(matches!(alpha.status, AccountStatus::Cooldown { .. }));
    assert_eq!(alpha.failure_count, 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_audit_ledger_chaos_integration() {
    let temp_dir = temp_vault_dir("audit_ledger");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_audited", "Audited Account", HashMap::new())
        .expect("registration should succeed");

    let fault_policy = AccountFaultPolicy::intermittent(AccountFault::SocketTimeout, 2);
    let mut manager = ResilientAccountManager::new(vault.clone(), fault_policy, 3);
    let mut ledger = AuditLedger::new();

    // 1. Phục hồi và chọn tài khoản thành công qua audit ledger
    let res = manager.select_and_record(&mut ledger, &AccountEngineKind::Antigravity);
    assert!(res.is_ok());
    assert_eq!(manager.resolved_count(), 1);
    assert!(manager.recovered_count() > 0);

    // 2. Kiểm tra tài khoản không tồn tại / pool trống bị chặn an toàn
    let res_empty = manager.select_and_record(&mut ledger, &AccountEngineKind::Codex);
    assert!(matches!(res_empty, Err(AccountError::NoAvailableAccount(_))));

    // 3. Kiểm tra tính toàn vẹn chuỗi băm của AuditLedger
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 2);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_audit_ledger_persistent_failure_records_exception() {
    let temp_dir = temp_vault_dir("audit_persistent");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));
    vault
        .register_account(AccountEngineKind::Codex, "acc_codex", "Codex Account", HashMap::new())
        .expect("registration should succeed");

    let fault_policy = AccountFaultPolicy::always(AccountFault::SocketTimeout);
    let mut manager = ResilientAccountManager::new(vault, fault_policy, 2);
    let mut ledger = AuditLedger::new();

    let res = manager.select_and_record(&mut ledger, &AccountEngineKind::Codex);
    assert_eq!(res, Err(AccountError::SocketTimeout));

    // Ledger phải ghi nhận sự kiện ngoại lệ bảo mật mức độ HighImpact
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 1);
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::HighImpact);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_high_volume_account_chaos_resilience() {
    let temp_dir = temp_vault_dir("high_volume");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));

    // Đăng ký 5 tài khoản
    for i in 0..5 {
        vault
            .register_account(
                AccountEngineKind::Antigravity,
                &format!("worker_{}", i),
                &format!("Worker {}", i),
                HashMap::new(),
            )
            .expect("register worker");
    }

    let fault_policy = AccountFaultPolicy::intermittent(AccountFault::ConnectionReset, 3);
    let mut manager = ResilientAccountManager::new(vault.clone(), fault_policy, 4);
    let mut ledger = AuditLedger::new();

    for _ in 0..40 {
        let res = manager.select_and_record(&mut ledger, &AccountEngineKind::Antigravity);
        assert!(res.is_ok());
    }

    assert_eq!(ledger.entries().len(), 40);
    assert!(ledger.verify_integrity().is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 40);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_account_pool_exhaustion_and_self_healing() {
    let temp_dir = temp_vault_dir("exhaustion_healing");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));

    vault
        .register_account(AccountEngineKind::Antigravity, "acc_1", "Account 1", HashMap::new())
        .expect("register 1");
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_2", "Account 2", HashMap::new())
        .expect("register 2");

    // Đưa cả 2 tài khoản vào cooldown giả lập
    vault.mark_cooldown(&AccountEngineKind::Antigravity, "acc_1", 300, "Simulated 429 RateLimit");
    vault.mark_cooldown(&AccountEngineKind::Antigravity, "acc_2", 300, "Simulated 429 RateLimit");

    let mut manager = ResilientAccountManager::with_vault(vault.clone());
    let res = manager.select_account(&AccountEngineKind::Antigravity);
    assert!(matches!(res, Err(AccountError::NoAvailableAccount(_))));

    // Kích hoạt cơ chế tự phục hồi (Self-Healing) qua reset cooldown
    assert!(vault.reset_cooldown(&AccountEngineKind::Antigravity, "acc_1"));

    let recovered = manager.select_account(&AccountEngineKind::Antigravity).expect("should heal");
    assert_eq!(recovered.id, "acc_1");
    assert_eq!(recovered.status, AccountStatus::Ready);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_vault_concurrent_chaos_and_lock_poisoning_recovery() {
    let temp_dir = temp_vault_dir("concurrent_chaos");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));

    // 1. Khởi tạo tài khoản
    vault
        .register_account(AccountEngineKind::Antigravity, "acc_init", "Init Account", HashMap::new())
        .expect("register init");

    // 2. Chạy 10 luồng đồng thời bốc tài khoản và đánh dấu cooldown
    let mut handles = Vec::new();
    for i in 0..10 {
        let v = vault.clone();
        handles.push(std::thread::spawn(move || {
            let id = format!("worker_acc_{}", i);
            let _ = v.register_account(AccountEngineKind::Antigravity, &id, &id, HashMap::new());
            if let Some(acc) = v.select_next_account(&AccountEngineKind::Antigravity) {
                if i % 2 == 0 {
                    v.mark_cooldown(&AccountEngineKind::Antigravity, &acc.id, 60, "Concurrent chaos cooldown");
                } else {
                    v.record_success(&AccountEngineKind::Antigravity, &acc.id);
                }
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join must not panic");
    }

    // Xác nhận vault vẫn hoạt động an toàn, không rò rỉ dữ liệu hoặc deadlock
    let accounts = vault.list_accounts(Some(&AccountEngineKind::Antigravity));
    assert!(accounts.len() >= 2);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_topic_b974be_31f26a_accounts_benchmark_and_resilience() {
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-b974be / 31f26a Accounts Benchmark...");

    let temp_dir = temp_vault_dir("benchmark_resilience");
    let vault = Arc::new(AccountVault::new_with_dir(temp_dir.clone()));

    for i in 0..5 {
        let _ = vault.register_account(
            AccountEngineKind::Antigravity,
            &format!("bench_worker_{}", i),
            &format!("Bench Worker {}", i),
            HashMap::new(),
        );
    }

    // 1. Single-threaded benchmark (10,000 requests)
    let mut manager = ResilientAccountManager::new(vault.clone(), AccountFaultPolicy::none(), 3)
        .with_max_capacity(2_000);

    let stats = AccountBenchmarkHarness::run_benchmark(&mut manager, 10_000);
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_resolved, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 500.0,
        "Throughput should exceed 500 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(
        stats.p50_latency_us > 0.0,
        "P50 latency must be tracked"
    );
    assert!(
        stats.p90_latency_us > 0.0,
        "P90 latency must be tracked"
    );
    assert!(
        stats.p99_latency_us < 20_000.0,
        "P99 latency must be under 20ms, got {} us",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 2. Verify bounded memory allocation (no unbounded leak)
    let mem_before = manager.memory_usage_bytes();
    // Run another 5,000 requests to trigger FIFO eviction in bounded capacity
    let _ = AccountBenchmarkHarness::run_benchmark(&mut manager, 5_000);
    let mem_after = manager.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 3. Batch account selection optimization test
    let batch_engines = vec![
        AccountEngineKind::Antigravity,
        AccountEngineKind::Antigravity,
        AccountEngineKind::Generic("missing_batch_engine".to_string()),
        AccountEngineKind::Antigravity,
    ];
    let batch_res = manager.select_batch(&batch_engines);
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(matches!(batch_res[2], Err(AccountError::NoAvailableAccount(_))));
    assert!(batch_res[3].is_ok());

    // 4. Concurrent benchmark test (10,000 requests across 4 threads)
    let concurrent_stats = AccountBenchmarkHarness::run_concurrent_benchmark(10_000, 4);
    assert_eq!(concurrent_stats.total_requests, 10_000);
    assert_eq!(concurrent_stats.successful_resolved, 9_000);
    assert_eq!(concurrent_stats.rejected_count, 1_000);
    assert!(
        concurrent_stats.throughput_ops_per_sec > 500.0,
        "Concurrent throughput should exceed 500 ops/sec, got {}",
        concurrent_stats.throughput_ops_per_sec
    );
    assert!(
        concurrent_stats.p50_latency_us > 0.0,
        "Concurrent P50 latency should be measured"
    );
    assert!(
        concurrent_stats.p90_latency_us > 0.0,
        "Concurrent P90 latency should be measured"
    );
    assert!(
        concurrent_stats.p99_latency_us < 20_000.0,
        "Concurrent P99 latency should be well under 20ms, got {} us",
        concurrent_stats.p99_latency_us
    );
    assert!(
        concurrent_stats.memory_allocated_bytes > 0,
        "Concurrent memory usage should be tracked"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
    println!("[Reproduction & Benchmark Test] TOPIC-20261105-b974be / 31f26a passed successfully!");
}

