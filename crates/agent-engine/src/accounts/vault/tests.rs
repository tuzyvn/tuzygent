use std::collections::HashMap;

use super::*;
use crate::accounts::types::AccountStatus;

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
        .register_account(AccountEngineKind::Antigravity, "acc1", "Account 1", env1)
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
    let accounts = vault.list_accounts(Some(&AccountEngineKind::Antigravity));
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "acc1");

    // Kiểm tra get_account không bị panic khi lock bị poisoned
    let acc1 = vault.get_account(&AccountEngineKind::Antigravity, "acc1");
    assert!(acc1.is_some());
    assert_eq!(
        acc1.expect("acc1 should exist after accounts lock poisoning")
            .name,
        "Account 1"
    );

    // Kiểm tra ghi / đăng ký tài khoản mới khi lock đang bị poisoned vẫn hoạt động bình thường
    let reg_res = vault.register_account(
        AccountEngineKind::Antigravity,
        "acc2",
        "Account 2",
        HashMap::new(),
    );
    assert!(
        reg_res.is_ok(),
        "register_account must safely acquire write lock and recover from poisoned state"
    );

    // Kiểm tra mark_cooldown, reset_cooldown, record_success khi lock bị poisoned
    vault.mark_cooldown(&AccountEngineKind::Antigravity, "acc1", 60, "Rate limited");
    let acc1_cooldown = vault
        .get_account(&AccountEngineKind::Antigravity, "acc1")
        .expect("acc1 should exist when checking cooldown");
    assert!(matches!(
        acc1_cooldown.status,
        AccountStatus::Cooldown { .. }
    ));

    assert!(vault.reset_cooldown(&AccountEngineKind::Antigravity, "acc1"));
    let acc1_reset = vault
        .get_account(&AccountEngineKind::Antigravity, "acc1")
        .expect("acc1 should exist after cooldown reset");
    assert_eq!(acc1_reset.status, AccountStatus::Ready);

    vault.record_success(&AccountEngineKind::Antigravity, "acc1");

    // Kiểm tra remove_account khi lock bị poisoned
    let removed = vault.remove_account(&AccountEngineKind::Antigravity, "acc2");
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
    let selected = vault.select_next_account(&AccountEngineKind::Antigravity);
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
fn test_recurring_system_error_cooldown_and_exhaustion_recovery() {
    let temp_dir =
        std::env::temp_dir().join(format!("vault_recurring_{}", uuid::Uuid::new_v4().simple()));
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    // 1. Đăng ký các tài khoản cho engine
    vault
        .register_account(
            AccountEngineKind::Antigravity,
            "acc1",
            "Account 1",
            HashMap::new(),
        )
        .expect("register acc1 should succeed");
    vault
        .register_account(
            AccountEngineKind::Antigravity,
            "acc2",
            "Account 2",
            HashMap::new(),
        )
        .expect("register acc2 should succeed");

    // 2. Tái hiện lỗi hệ thống lặp lại (recurring rate limits/auth failures)
    vault.mark_cooldown(
        &AccountEngineKind::Antigravity,
        "acc1",
        300,
        "429 RateLimitExceeded",
    );
    let acc1 = vault
        .get_account(&AccountEngineKind::Antigravity, "acc1")
        .expect("acc1 should exist after marking cooldown");
    assert_eq!(acc1.failure_count, 1);
    assert!(matches!(acc1.status, AccountStatus::Cooldown { .. }));

    // Hệ thống tự động chuyển tiếp sang tài khoản khả dụng acc2
    let picked = vault
        .select_next_account(&AccountEngineKind::Antigravity)
        .expect("acc2 should be available");
    assert_eq!(picked.id, "acc2");

    // acc2 tiếp tục gặp lỗi lặp lại -> toàn bộ account pool bị cooldown
    vault.mark_cooldown(
        &AccountEngineKind::Antigravity,
        "acc2",
        300,
        "429 RateLimitExceeded",
    );
    let acc2 = vault
        .get_account(&AccountEngineKind::Antigravity, "acc2")
        .expect("acc2 should exist after marking cooldown");
    assert_eq!(acc2.failure_count, 1);
    assert!(matches!(acc2.status, AccountStatus::Cooldown { .. }));

    // Khi toàn bộ tài khoản bị lỗi lặp lại, hệ thống xử lý an toàn (trả về None mà không panic hay crash)
    assert!(vault
        .select_next_account(&AccountEngineKind::Antigravity)
        .is_none());

    // 3. Kiểm tra cơ chế tự phục hồi (Self-Healing) khi cooldown được reset hoặc hết hạn
    assert!(vault.reset_cooldown(&AccountEngineKind::Antigravity, "acc1"));
    let recovered = vault
        .select_next_account(&AccountEngineKind::Antigravity)
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
            .get_account(&AccountEngineKind::Antigravity, "non_existent_id")
            .is_none(),
        "get_account on empty vault must return None"
    );

    // 2. Đăng ký một tài khoản cho Antigravity
    vault
        .register_account(
            AccountEngineKind::Antigravity,
            "acc1",
            "Account 1",
            HashMap::new(),
        )
        .expect("register acc1 should succeed");

    // 3. Gọi get_account với ID không tồn tại trên engine đã có dữ liệu
    assert!(
        vault
            .get_account(&AccountEngineKind::Antigravity, "non_existent_id")
            .is_none(),
        "get_account with unknown id on existing engine must return None safely without panic"
    );
    assert!(
        vault
            .get_account(&AccountEngineKind::Antigravity, "")
            .is_none(),
        "get_account with empty string id must return None safely"
    );

    // 4. Gọi get_account với engine khác chưa có tài khoản nào
    assert!(
        vault
            .get_account(&AccountEngineKind::Codex, "acc1")
            .is_none(),
        "get_account with different engine must return None safely"
    );

    // 5. Kiểm tra tài khoản tồn tại thực tế vẫn trả về Some
    let acc1 = vault.get_account(&AccountEngineKind::Antigravity, "acc1");
    assert!(acc1.is_some());
    assert_eq!(acc1.expect("acc1 should exist").id, "acc1");

    // Dọn dẹp thư mục tạm
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
    assert!(validate_engine_name("antigravity").is_ok());
    assert!(validate_engine_name("../evil").is_err());
    assert!(validate_engine_name("evil/name").is_err());

    // 3. Kiểm tra build_account_env & build_env_for_account cho các engine
    let temp_dir =
        std::env::temp_dir().join(format!("vault_env_{}", uuid::Uuid::new_v4().simple()));
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    let mut extra = HashMap::new();
    extra.insert("CUSTOM_KEY".to_string(), "custom_val".to_string());

    let acc_anti = vault
        .register_account(
            AccountEngineKind::Antigravity,
            "anti_user",
            "Anti User",
            extra.clone(),
        )
        .expect("register antigravity should succeed");
    let env_anti = vault.build_env_for_account(&acc_anti);
    assert_eq!(
        env_anti.get("ANTIGRAVITY_HOME"),
        Some(&acc_anti.home_dir.to_string_lossy().to_string())
    );
    assert_eq!(
        env_anti.get("ANTIGRAVITY_CONFIG_DIR"),
        Some(&acc_anti.home_dir.to_string_lossy().to_string())
    );
    assert_eq!(env_anti.get("CUSTOM_KEY"), Some(&"custom_val".to_string()));

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

    // 4. import_local_credentials cho tài khoản không tồn tại trả về NotFound
    assert!(vault
        .import_local_credentials(&AccountEngineKind::Antigravity, "non_existent")
        .is_err());

    let _ = std::fs::remove_dir_all(temp_dir);
}
