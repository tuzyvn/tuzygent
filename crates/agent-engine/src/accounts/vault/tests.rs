use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::accounts::types::{AccountProfile, AccountStatus};

#[test]
fn test_default_dir_uses_home_directory_and_accounts_suffix() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());

    assert_eq!(
        AccountVault::default_dir(),
        std::path::Path::new(&home)
            .join(".agentcore")
            .join("accounts")
    );
}

#[test]
fn test_new_and_default_use_default_directory() {
    let expected_dir = AccountVault::default_dir();

    let from_new = AccountVault::new();
    assert_eq!(from_new.base_dir(), expected_dir);

    let from_default = AccountVault::default();
    assert_eq!(from_default.base_dir(), expected_dir);
}

#[test]
fn test_base_dir_preserves_the_configured_path() {
    let configured_dir = std::path::PathBuf::from("vault-data")
        .join("..")
        .join("accounts");
    let vault = AccountVault::new_with_dir(configured_dir.clone());

    assert_eq!(vault.base_dir(), configured_dir.as_path());
}

#[test]
fn test_accounts_read_returns_registered_profiles() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_accounts_read_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "read-account",
            "Read account",
            HashMap::new(),
        )
        .expect("register account");

    let accounts = vault.accounts_read();

    let registered = accounts
        .get(&AccountEngineKind::Codex)
        .expect("engine account list should exist");
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].id, "read-account");
    drop(accounts);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_rr_read_observes_round_robin_index() {
    let temp_dir =
        std::env::temp_dir().join(format!("vault_rr_read_{}", uuid::Uuid::new_v4().simple()));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    let engine = AccountEngineKind::Codex;
    {
        let mut indices = vault.rr_write();
        indices.insert(engine.clone(), AtomicUsize::new(7));
    }

    let indices = vault.rr_read();

    assert_eq!(
        indices
            .get(&engine)
            .expect("round-robin index should exist")
            .load(Ordering::SeqCst),
        7
    );
    drop(indices);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_rr_write_updates_round_robin_index() {
    let temp_dir =
        std::env::temp_dir().join(format!("vault_rr_write_{}", uuid::Uuid::new_v4().simple()));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    let engine = AccountEngineKind::Claude;
    {
        let mut indices = vault.rr_write();
        indices.insert(engine.clone(), AtomicUsize::new(0));
        indices
            .get(&engine)
            .expect("inserted round-robin index should exist")
            .store(3, Ordering::SeqCst);
    }

    assert_eq!(
        vault
            .rr_read()
            .get(&engine)
            .expect("round-robin index should remain readable")
            .load(Ordering::SeqCst),
        3
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_vault_public_storage_and_copy_helpers() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_public_helpers_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let source = temp_dir.join("source");
    let destination = temp_dir.join("destination");
    std::fs::create_dir_all(source.join("nested")).expect("create source directory");
    std::fs::write(source.join("credentials.json"), "{\"token\":\"secret\"}")
        .expect("write credential file");
    std::fs::write(source.join("nested").join("auth.data"), "auth")
        .expect("write nested credential file");
    std::fs::write(source.join("notes.txt"), "not a credential").expect("write ordinary file");

    copy_credential_dir(&source, &destination).expect("copy credential directory");
    assert_eq!(
        std::fs::read_to_string(destination.join("credentials.json")).unwrap(),
        "{\"token\":\"secret\"}"
    );
    assert_eq!(
        std::fs::read_to_string(destination.join("nested").join("auth.data")).unwrap(),
        "auth"
    );
    assert!(!destination.join("notes.txt").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(destination.join("credentials.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let wrapper_destination = temp_dir.join("wrapper-destination");
    copy_dir_recursive(&source, &wrapper_destination).expect("copy through recursive helper");
    assert_eq!(
        std::fs::read_to_string(wrapper_destination.join("credentials.json")).unwrap(),
        "{\"token\":\"secret\"}"
    );
    assert!(!wrapper_destination.join("notes.txt").exists());
    assert!(copy_dir_recursive(
        &temp_dir.join("missing-source"),
        &temp_dir.join("failed-copy")
    )
    .is_err());

    let base_dir = temp_dir.join("metadata");
    let vault = AccountVault::new_with_dir(base_dir.clone());
    let metadata_file = base_dir.join("accounts.json");
    assert_eq!(vault.metadata_file(), metadata_file);

    assert!(load_profiles_from_file(&metadata_file)
        .expect("missing metadata should load as empty")
        .is_empty());
    let profile = AccountProfile {
        id: "saved-account".to_string(),
        name: "Saved account".to_string(),
        engine: AccountEngineKind::Codex,
        home_dir: base_dir.join("codex").join("saved-account"),
        status: AccountStatus::Ready,
        requests_count: 3,
        failure_count: 1,
        last_used_at: None,
        extra_env: HashMap::from([("TOKEN".to_string(), "secret".to_string())]),
    };
    save_profiles_to_file(&base_dir, &metadata_file, std::slice::from_ref(&profile))
        .expect("save account profiles");
    assert_eq!(
        load_profiles_from_file(&metadata_file).expect("load saved profiles"),
        vec![profile]
    );

    std::fs::write(&metadata_file, "not valid JSON").expect("write malformed metadata");
    assert_eq!(
        load_profiles_from_file(&metadata_file).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    let invalid_meta_target = temp_dir.join("missing-parent").join("accounts.json");
    assert!(save_profiles_to_file(&base_dir, &invalid_meta_target, &[]).is_err());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_lock_safety() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_lock_safety_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    // 1. Đăng ký tài khoản ban đầu
    let mut env1 = HashMap::new();
    env1.insert("KEY".to_string(), "val".to_string());
    vault
        .register_account(AccountEngineKind::Codex, "acc1", "Account 1", env1)
        .expect("initial registration should succeed");

    // 2. Mô phỏng kịch bản RwLock `self.accounts` bị poisoned
    // Một thread panic khi đang giữ write guard trên accounts
    let accounts_lock = vault.accounts.clone();
    let handle = std::thread::spawn(move || {
        let _guard = accounts_lock
            .write()
            .expect("failed to acquire accounts write lock in poison simulation thread");
        std::panic::panic_any("Simulated worker panic while holding accounts write lock");
    });
    assert!(
        handle.join().is_err(),
        "Thread must panic and poison the lock"
    );
    assert!(
        vault.accounts.is_poisoned(),
        "accounts RwLock must now be in poisoned state"
    );

    // Kiểm tra xử lý an toàn: đọc danh sách không bị panic, dữ liệu vẫn được bảo toàn
    let accounts = vault.list_accounts(Some(&AccountEngineKind::Codex));
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "acc1");

    // Kiểm tra get_account không bị panic khi lock bị poisoned
    let acc1 = vault.get_account(&AccountEngineKind::Codex, "acc1");
    assert!(acc1.is_some());
    assert_eq!(
        acc1.expect("acc1 should exist after accounts lock poisoning")
            .name,
        "Account 1"
    );

    // Kiểm tra ghi / đăng ký tài khoản mới khi lock đang bị poisoned vẫn hoạt động bình thường
    let reg_res = vault.register_account(
        AccountEngineKind::Codex,
        "acc2",
        "Account 2",
        HashMap::new(),
    );
    assert!(
        reg_res.is_ok(),
        "register_account must safely acquire write lock and recover from poisoned state"
    );

    // Kiểm tra mark_cooldown, reset_cooldown, record_success khi lock bị poisoned
    vault.mark_cooldown(&AccountEngineKind::Codex, "acc1", 60, "Rate limited");
    let acc1_cooldown = vault
        .get_account(&AccountEngineKind::Codex, "acc1")
        .expect("acc1 should exist when checking cooldown");
    assert!(matches!(
        acc1_cooldown.status,
        AccountStatus::Cooldown { .. }
    ));

    assert!(vault.reset_cooldown(&AccountEngineKind::Codex, "acc1"));
    let acc1_reset = vault
        .get_account(&AccountEngineKind::Codex, "acc1")
        .expect("acc1 should exist after cooldown reset");
    assert_eq!(acc1_reset.status, AccountStatus::Ready);

    vault.record_success(&AccountEngineKind::Codex, "acc1");

    // Kiểm tra remove_account khi lock bị poisoned
    let removed = vault.remove_account(&AccountEngineKind::Codex, "acc2");
    assert!(removed.expect("remove_account should succeed and return whether account was removed"));

    // Kiểm tra save_sync khi lock bị poisoned
    assert!(vault.save_sync().is_ok());

    // 3. Mô phỏng kịch bản RwLock `self.round_robin_indices` bị poisoned
    let rr_lock = vault.round_robin_indices.clone();
    let handle_rr = std::thread::spawn(move || {
        let _guard = rr_lock
            .write()
            .expect("failed to acquire round_robin_indices write lock in poison simulation thread");
        std::panic::panic_any("Simulated worker panic while holding round_robin_indices lock");
    });
    assert!(
        handle_rr.join().is_err(),
        "Thread must panic and poison the rr lock"
    );
    assert!(
        vault.round_robin_indices.is_poisoned(),
        "round_robin_indices RwLock must now be in poisoned state"
    );

    // Kiểm tra select_next_account xử lý an toàn: đọc và ghi rr_indices mà không panic
    let selected = vault.select_next_account(&AccountEngineKind::Codex);
    assert!(
        selected.is_some(),
        "select_next_account must safely recover from poisoned round_robin_indices lock"
    );
    assert_eq!(
        selected
            .expect("select_next_account should return an account after rr lock poisoning")
            .id,
        "acc1"
    );

    // Dọn dẹp thư mục tạm
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_vault_clears_account_lock_poison_after_recovery() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_poison_recovery_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    let accounts_lock = vault.accounts.clone();

    let handle = std::thread::spawn(move || {
        let _guard = accounts_lock
            .write()
            .expect("poison simulation should acquire accounts write lock");
        panic!("simulate a worker panic while holding accounts lock");
    });
    assert!(
        handle.join().is_err(),
        "worker should panic and poison lock"
    );
    assert!(
        vault.accounts.is_poisoned(),
        "lock should be poisoned initially"
    );

    // A normal vault operation recovers the poisoned guard. Recovery should also
    // clear the poison marker so later callers do not inherit stale poison state.
    assert!(vault.list_accounts(None).is_empty());
    assert!(
        !vault.accounts.is_poisoned(),
        "successful vault recovery should clear the accounts lock poison marker"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_recurring_system_error_cooldown_and_exhaustion_recovery() {
    let temp_dir =
        std::env::temp_dir().join(format!("vault_recurring_{}", uuid::Uuid::new_v4().simple()));
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    // 1. Đăng ký các tài khoản cho engine
    vault
        .register_account(
            AccountEngineKind::Codex,
            "acc1",
            "Account 1",
            HashMap::new(),
        )
        .expect("register acc1 should succeed");
    vault
        .register_account(
            AccountEngineKind::Codex,
            "acc2",
            "Account 2",
            HashMap::new(),
        )
        .expect("register acc2 should succeed");

    // 2. Tái hiện lỗi hệ thống lặp lại (recurring rate limits/auth failures)
    vault.mark_cooldown(
        &AccountEngineKind::Codex,
        "acc1",
        300,
        "429 RateLimitExceeded",
    );
    let acc1 = vault
        .get_account(&AccountEngineKind::Codex, "acc1")
        .expect("acc1 should exist after marking cooldown");
    assert_eq!(acc1.failure_count, 1);
    assert!(matches!(acc1.status, AccountStatus::Cooldown { .. }));

    // Hệ thống tự động chuyển tiếp sang tài khoản khả dụng acc2
    let picked = vault
        .select_next_account(&AccountEngineKind::Codex)
        .expect("acc2 should be available");
    assert_eq!(picked.id, "acc2");

    // acc2 tiếp tục gặp lỗi lặp lại -> toàn bộ account pool bị cooldown
    vault.mark_cooldown(
        &AccountEngineKind::Codex,
        "acc2",
        300,
        "429 RateLimitExceeded",
    );
    let acc2 = vault
        .get_account(&AccountEngineKind::Codex, "acc2")
        .expect("acc2 should exist after marking cooldown");
    assert_eq!(acc2.failure_count, 1);
    assert!(matches!(acc2.status, AccountStatus::Cooldown { .. }));

    // Khi toàn bộ tài khoản bị lỗi lặp lại, hệ thống xử lý an toàn (trả về None mà không panic hay crash)
    assert!(vault
        .select_next_account(&AccountEngineKind::Codex)
        .is_none());

    // 3. Kiểm tra cơ chế tự phục hồi (Self-Healing) khi cooldown được reset hoặc hết hạn
    assert!(vault.reset_cooldown(&AccountEngineKind::Codex, "acc1"));
    let recovered = vault
        .select_next_account(&AccountEngineKind::Codex)
        .expect("acc1 should be available after recovery");
    assert_eq!(recovered.id, "acc1");
    assert_eq!(recovered.failure_count, 0);

    // Dọn dẹp thư mục tạm
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_get_account_non_existent_returns_none() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_non_existent_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    // 1. Khi vault chưa có bất kỳ tài khoản hoặc engine nào
    assert!(
        vault
            .get_account(&AccountEngineKind::Codex, "non_existent_id")
            .is_none(),
        "get_account on empty vault must return None"
    );

    // 2. Đăng ký một tài khoản cho Codex
    vault
        .register_account(
            AccountEngineKind::Codex,
            "acc1",
            "Account 1",
            HashMap::new(),
        )
        .expect("register acc1 should succeed");

    // 3. Gọi get_account với ID không tồn tại trên engine đã có dữ liệu
    assert!(
        vault
            .get_account(&AccountEngineKind::Codex, "non_existent_id")
            .is_none(),
        "get_account with unknown id on existing engine must return None safely without panic"
    );
    assert!(
        vault
            .get_account(&AccountEngineKind::Codex, "")
            .is_none(),
        "get_account with empty string id must return None safely"
    );

    // 4. Gọi get_account với engine khác chưa có tài khoản nào
    assert!(
        vault
            .get_account(&AccountEngineKind::Claude, "acc1")
            .is_none(),
        "get_account with different engine must return None safely"
    );

    // 5. Kiểm tra tài khoản tồn tại thực tế vẫn trả về Some
    let acc1 = vault.get_account(&AccountEngineKind::Codex, "acc1");
    assert!(acc1.is_some());
    assert_eq!(acc1.expect("acc1 should exist").id, "acc1");

    // Dọn dẹp thư mục tạm
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_save_sync_persists_current_profile_state() {
    let temp_dir =
        std::env::temp_dir().join(format!("vault_save_sync_{}", uuid::Uuid::new_v4().simple()));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "persisted-account",
            "Persisted account",
            HashMap::new(),
        )
        .expect("register account");

    // Change a persisted field in memory so this checks save_sync, not only registration.
    vault
        .accounts_write()
        .get_mut(&AccountEngineKind::Codex)
        .expect("registered engine exists")[0]
        .requests_count = 7;
    vault.save_sync().expect("save current vault state");

    let reloaded = AccountVault::new_with_dir(temp_dir.clone());
    let profile = reloaded
        .get_account(&AccountEngineKind::Codex, "persisted-account")
        .expect("saved profile should be loaded by a new vault");
    assert_eq!(profile.requests_count, 7);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_select_next_account_releases_expired_cooldown() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_expired_cooldown_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "expired-account",
            "Expired account",
            HashMap::new(),
        )
        .expect("register account");
    vault
        .accounts_write()
        .get_mut(&AccountEngineKind::Codex)
        .expect("registered engine exists")[0]
        .status = AccountStatus::Cooldown {
        until: chrono::Utc::now() - chrono::Duration::seconds(1),
        reason: "expired test cooldown".to_string(),
    };

    let selected = vault
        .select_next_account(&AccountEngineKind::Codex)
        .expect("expired cooldown account should be selected");
    assert_eq!(selected.id, "expired-account");
    assert_eq!(selected.status, AccountStatus::Ready);
    assert_eq!(
        vault
            .get_account(&AccountEngineKind::Codex, "expired-account")
            .expect("account remains in vault")
            .status,
        AccountStatus::Ready
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_select_next_account_skips_disabled_accounts() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_disabled_account_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "disabled-account",
            "Disabled account",
            HashMap::new(),
        )
        .expect("register disabled account");
    vault
        .register_account(
            AccountEngineKind::Codex,
            "ready-account",
            "Ready account",
            HashMap::new(),
        )
        .expect("register ready account");

    vault
        .accounts_write()
        .get_mut(&AccountEngineKind::Codex)
        .expect("registered engine exists")[0]
        .status = AccountStatus::Disabled;

    let selected = vault
        .select_next_account(&AccountEngineKind::Codex)
        .expect("ready account should be selected");
    assert_eq!(selected.id, "ready-account");
    assert_eq!(selected.requests_count, 1);
    assert_eq!(
        vault
            .get_account(&AccountEngineKind::Codex, "disabled-account")
            .expect("disabled account remains registered")
            .requests_count,
        0
    );

    vault
        .accounts_write()
        .get_mut(&AccountEngineKind::Codex)
        .expect("registered engine exists")[1]
        .status = AccountStatus::Disabled;
    assert!(vault
        .select_next_account(&AccountEngineKind::Codex)
        .is_none());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_record_success_clears_failure_state() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_record_success_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "recovered-account",
            "Recovered account",
            HashMap::new(),
        )
        .expect("register account");
    vault.mark_cooldown(
        &AccountEngineKind::Codex,
        "recovered-account",
        60,
        "rate limited",
    );

    vault.record_success(&AccountEngineKind::Codex, "recovered-account");

    let profile = vault
        .get_account(&AccountEngineKind::Codex, "recovered-account")
        .expect("account remains in vault");
    assert_eq!(profile.failure_count, 0);
    assert_eq!(profile.status, AccountStatus::Ready);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_mark_cooldown_updates_and_persists_failure_state() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_mark_cooldown_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "limited-account",
            "Limited account",
            HashMap::new(),
        )
        .expect("register account");

    vault.mark_cooldown(
        &AccountEngineKind::Codex,
        "limited-account",
        60,
        "rate limited",
    );

    let profile = vault
        .get_account(&AccountEngineKind::Codex, "limited-account")
        .expect("account remains in vault");
    assert_eq!(profile.failure_count, 1);
    assert!(matches!(
        &profile.status,
        AccountStatus::Cooldown { until, reason }
            if *until > chrono::Utc::now() && reason == "rate limited"
    ));

    let reloaded = AccountVault::new_with_dir(temp_dir.clone());
    let persisted = reloaded
        .get_account(&AccountEngineKind::Codex, "limited-account")
        .expect("cooldown state should be persisted");
    assert_eq!(persisted.failure_count, 1);
    assert!(matches!(
        &persisted.status,
        AccountStatus::Cooldown { reason, .. } if reason == "rate limited"
    ));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_reset_cooldown_returns_false_for_unknown_account() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_reset_missing_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "existing-account",
            "Existing account",
            HashMap::new(),
        )
        .expect("register account");
    vault.mark_cooldown(
        &AccountEngineKind::Codex,
        "existing-account",
        60,
        "rate limited",
    );

    assert!(!vault.reset_cooldown(&AccountEngineKind::Codex, "unknown-account"));
    assert!(!vault.reset_cooldown(&AccountEngineKind::Claude, "existing-account"));
    let profile = vault
        .get_account(&AccountEngineKind::Codex, "existing-account")
        .expect("unknown reset requests must leave the real account untouched");
    assert_eq!(profile.failure_count, 1);
    assert!(matches!(profile.status, AccountStatus::Cooldown { .. }));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_reset_cooldown_clears_failure_state_and_persists_it() {
    let temp_dir = std::env::temp_dir().join(format!(
        "vault_reset_cooldown_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let vault = AccountVault::new_with_dir(temp_dir.clone());
    vault
        .register_account(
            AccountEngineKind::Codex,
            "cooldown-account",
            "Cooldown account",
            HashMap::new(),
        )
        .expect("register account");
    vault.mark_cooldown(
        &AccountEngineKind::Codex,
        "cooldown-account",
        60,
        "rate limited",
    );

    assert!(vault.reset_cooldown(&AccountEngineKind::Codex, "cooldown-account"));

    let reloaded = AccountVault::new_with_dir(temp_dir.clone());
    let profile = reloaded
        .get_account(&AccountEngineKind::Codex, "cooldown-account")
        .expect("reset profile should be persisted");
    assert_eq!(profile.status, AccountStatus::Ready);
    assert_eq!(profile.failure_count, 0);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_env_builder_and_ops_helpers() {
    // 1. Kiểm tra helper sanitize_account_id
    assert!(sanitize_account_id("").is_err());
    assert!(sanitize_account_id(".").is_err());
    assert!(sanitize_account_id("..").is_err());
    assert!(sanitize_account_id("../etc").is_err());
    assert!(sanitize_account_id("foo/bar").is_err());
    assert!(sanitize_account_id("foo\\bar").is_err());
    assert!(sanitize_account_id("bad@char").is_err());
    assert_eq!(
        sanitize_account_id("  My Account-1 ").as_deref().ok(),
        Some("my_account-1")
    );

    // 2. Kiểm tra helper validate_engine_name
    assert!(validate_engine_name("codex").is_ok());
    assert!(validate_engine_name("claude").is_ok());
    assert_eq!(
        validate_engine_name("antigravity").unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        validate_engine_name("AGY").unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        validate_engine_name("gemini").unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(validate_engine_name("../evil").is_err());
    assert!(validate_engine_name("evil/name").is_err());

    // 3. Kiểm tra build_account_env & build_env_for_account cho các engine
    let temp_dir =
        std::env::temp_dir().join(format!("vault_env_{}", uuid::Uuid::new_v4().simple()));
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    let mut extra = HashMap::new();
    extra.insert("CUSTOM_KEY".to_string(), "custom_val".to_string());

    let google_reg = vault.register_account(
        AccountEngineKind::from_str_name("antigravity"),
        "anti_user",
        "Anti User",
        extra.clone(),
    );
    assert_eq!(
        google_reg.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );

    let acc_codex = vault
        .register_account(
            AccountEngineKind::Codex,
            "codex_user",
            "Codex User",
            extra.clone(),
        )
        .expect("register codex should succeed");
    let env_codex = build_account_env(&acc_codex);
    assert_eq!(
        env_codex.get("CODEX_HOME"),
        Some(&acc_codex.home_dir.to_string_lossy().to_string())
    );
    assert_eq!(
        env_codex.get("CODEX_CONFIG_DIR"),
        Some(&acc_codex.home_dir.to_string_lossy().to_string())
    );
    assert_eq!(env_codex.get("CUSTOM_KEY"), Some(&"custom_val".to_string()));

    let acc_claude = vault
        .register_account(
            AccountEngineKind::Claude,
            "claude_user",
            "Claude User",
            extra.clone(),
        )
        .expect("register claude should succeed");
    let env_claude = vault.build_env_for_account(&acc_claude);
    assert_eq!(
        env_claude.get("CLAUDE_CONFIG_DIR"),
        Some(&acc_claude.home_dir.to_string_lossy().to_string())
    );

    let acc_generic = vault
        .register_account(
            AccountEngineKind::Generic("custom_engine".into()),
            "gen_user",
            "Gen User",
            extra,
        )
        .expect("register generic should succeed");
    let env_generic = vault.build_env_for_account(&acc_generic);
    assert_eq!(
        env_generic.get("HOME"),
        Some(&acc_generic.home_dir.to_string_lossy().to_string())
    );

    // 4. import_local_credentials từ chối Google/Antigravity (PermissionDenied) và báo NotFound nếu không tồn tại
    assert_eq!(
        vault
            .import_local_credentials(&AccountEngineKind::from_str_name("antigravity"), "any")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(vault
        .import_local_credentials(&AccountEngineKind::Codex, "non_existent")
        .is_err());

    let _ = std::fs::remove_dir_all(temp_dir);
}
