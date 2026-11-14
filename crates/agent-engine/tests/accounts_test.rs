use agent_engine::{AccountEngineKind, AccountVault};
use std::collections::HashMap;

fn temp_vault_dir(prefix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vault_test_{}_{}",
        prefix,
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_account_vault_registration_and_isolation() {
    let temp_dir = temp_vault_dir("register");
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    // 1. Đăng ký tài khoản Codex 1
    let mut env1 = HashMap::new();
    env1.insert("OPENAI_ORG".to_string(), "org-alpha".to_string());
    let acc1 = vault
        .register_account(
            AccountEngineKind::Codex,
            "acc_codex_work",
            "Work Codex Account",
            env1,
        )
        .unwrap();

    // 2. Đăng ký tài khoản Codex 2
    let mut env2 = HashMap::new();
    env2.insert("OPENAI_ORG".to_string(), "org-beta".to_string());
    let acc2 = vault
        .register_account(
            AccountEngineKind::Codex,
            "acc_codex_personal",
            "Personal Codex Account",
            env2,
        )
        .unwrap();

    // Google/Antigravity multi-account registration must be rejected by policy
    let google_err = vault
        .register_account(
            AccountEngineKind::from_str_name("antigravity"),
            "acc_google",
            "Google",
            HashMap::new(),
        )
        .unwrap_err();
    assert_eq!(google_err.kind(), std::io::ErrorKind::PermissionDenied);

    // Kiểm tra thư mục sandbox cách ly riêng biệt
    assert_ne!(acc1.home_dir, acc2.home_dir);
    assert!(acc1.home_dir.ends_with("codex/acc_codex_work"));
    assert!(acc2.home_dir.ends_with("codex/acc_codex_personal"));

    // Kiểm tra biến môi trường được inject khi spawn CLI
    let env_acc1 = vault.build_env_for_account(&acc1);
    assert_eq!(
        env_acc1.get("HOME"),
        Some(&acc1.home_dir.to_string_lossy().to_string())
    );
    assert_eq!(
        env_acc1.get("CODEX_HOME"),
        Some(&acc1.home_dir.to_string_lossy().to_string())
    );
    assert_eq!(env_acc1.get("OPENAI_ORG"), Some(&"org-alpha".to_string()));

    let env_acc2 = vault.build_env_for_account(&acc2);
    assert_eq!(
        env_acc2.get("HOME"),
        Some(&acc2.home_dir.to_string_lossy().to_string())
    );
    assert_eq!(env_acc2.get("OPENAI_ORG"), Some(&"org-beta".to_string()));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_round_robin_rotation_and_cooldown() {
    let temp_dir = temp_vault_dir("rotation");
    let vault = AccountVault::new_with_dir(temp_dir.clone());

    // Đăng ký 2 tài khoản Codex
    let mut env_a = HashMap::new();
    env_a.insert("OPENAI_API_KEY".to_string(), "sk-account-a".to_string());
    vault
        .register_account(AccountEngineKind::Codex, "acc_a", "Codex Account A", env_a)
        .unwrap();

    let mut env_b = HashMap::new();
    env_b.insert("OPENAI_API_KEY".to_string(), "sk-account-b".to_string());
    vault
        .register_account(AccountEngineKind::Codex, "acc_b", "Codex Account B", env_b)
        .unwrap();

    // Lượt 1: Bốc acc_a
    let picked1 = vault
        .select_next_account(&AccountEngineKind::Codex)
        .unwrap();
    assert_eq!(picked1.id, "acc_a");

    // Lượt 2: Xoay vòng sang acc_b
    let picked2 = vault
        .select_next_account(&AccountEngineKind::Codex)
        .unwrap();
    assert_eq!(picked2.id, "acc_b");

    // Lượt 3: Xoay vòng lại acc_a
    let picked3 = vault
        .select_next_account(&AccountEngineKind::Codex)
        .unwrap();
    assert_eq!(picked3.id, "acc_a");

    // Giả sử acc_a gặp lỗi Rate Limit 429 -> Đưa vào Cooldown 60s
    vault.mark_cooldown(
        &AccountEngineKind::Codex,
        "acc_a",
        60,
        "Rate limit 429 exceeded",
    );

    // Lượt 4: Vì acc_a đang Cooldown, hệ thống tự động bỏ qua acc_a và chỉ bốc acc_b
    let picked4 = vault
        .select_next_account(&AccountEngineKind::Codex)
        .unwrap();
    assert_eq!(picked4.id, "acc_b");

    // Lượt 5: Vẫn tiếp tục bốc acc_b vì acc_a chưa hết thời gian phạt
    let picked5 = vault
        .select_next_account(&AccountEngineKind::Codex)
        .unwrap();
    assert_eq!(picked5.id, "acc_b");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_vault_persistence_reload() {
    let temp_dir = temp_vault_dir("persistence");

    {
        let vault = AccountVault::new_with_dir(temp_dir.clone());
        vault
            .register_account(
                AccountEngineKind::Codex,
                "persist_acc",
                "Persistent Account",
                HashMap::new(),
            )
            .unwrap();
    }

    // Khởi tạo lại vault từ cùng thư mục để kiểm tra tính bền vững
    {
        let reloaded = AccountVault::new_with_dir(temp_dir.clone());
        let accounts = reloaded.list_accounts(Some(&AccountEngineKind::Codex));
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "persist_acc");
        assert_eq!(accounts[0].name, "Persistent Account");
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}
