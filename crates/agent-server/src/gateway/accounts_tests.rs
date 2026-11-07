use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use agent_engine::AccountEngineKind;
use agent_runtime::{AgentSupervisor, InterAgentBus};
use axum::extract::State;

fn create_test_state() -> Result<AppState, Box<dyn std::error::Error>> {
    let bus = InterAgentBus::new(16);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));
    let temp_dir =
        std::env::temp_dir().join(format!("gateway_test_{}", uuid::Uuid::new_v4().simple()));
    let vault = Arc::new(agent_engine::AccountVault::new_with_dir(temp_dir));
    let task_store = Arc::new(agent_runtime::SqliteTaskStore::memory()?);
    let task_service = Arc::new(agent_runtime::TaskService::new(
        task_store, "/tmp", 2, None, None,
    ));

    Ok(AppState {
        supervisor,
        bus,
        workspace: "/tmp".into(),
        vault,
        task_service,
        metrics: Arc::new(crate::metrics::ResilientMetricsService::default()),
    })
}

fn register_test_account(
    state: &AppState,
    engine: &str,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    state.vault.register_account(
        AccountEngineKind::from_str_name(engine),
        id,
        "Gateway test account",
        HashMap::new(),
    )?;
    Ok(())
}

fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
    if let Some(value) = value {
        std::env::set_var(key, value);
    } else {
        std::env::remove_var(key);
    }
}

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn test_gateway_account_profile_sanitization() -> Result<(), Box<dyn std::error::Error>> {
    let state = create_test_state()?;

    let mut env = HashMap::new();
    env.insert(
        "SECRET_API_KEY".to_string(),
        "sk-super-secret-12345".to_string(),
    );
    env.insert("ANTHROPIC_AUTH".to_string(), "token-999".to_string());

    let req = AddAccountReq {
        engine: "antigravity".to_string(),
        id: "safe-acc-01".to_string(),
        name: "Sanitized Test Account".to_string(),
        extra_env: env,
    };

    // 1. Add account and check returned profile is sanitized (SafeAccountProfile)
    let add_res = add_account_handler(State(state.clone()), Json(req)).await;
    assert!(add_res.is_ok());
    let safe_profile = add_res.map_err(|(code, msg)| format!("{code}: {msg}"))?.0;
    assert_eq!(safe_profile.id, "safe-acc-01");
    assert_eq!(safe_profile.name, "Sanitized Test Account");
    assert!(safe_profile
        .extra_env_keys
        .contains(&"SECRET_API_KEY".to_string()));
    assert!(safe_profile
        .extra_env_keys
        .contains(&"ANTHROPIC_AUTH".to_string()));

    // Serialized representation must not leak the raw secret value
    let serialized = serde_json::to_string(&safe_profile)?;
    assert!(!serialized.contains("sk-super-secret-12345"));
    assert!(!serialized.contains("token-999"));

    // 2. Listing accounts also returns sanitized profiles
    let list_res = list_accounts_handler(State(state)).await;
    let list_json = list_res.0;
    let list_str = list_json.to_string();
    assert!(!list_str.contains("sk-super-secret-12345"));
    assert!(!list_str.contains("token-999"));
    assert!(list_str.contains("safe-acc-01"));

    Ok(())
}

#[tokio::test]
async fn test_reset_account_handler_reports_reset_and_missing_accounts(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = create_test_state()?;
    register_test_account(&state, "antigravity", "reset-me")?;
    state.vault.mark_cooldown(
        &AccountEngineKind::Antigravity,
        "reset-me",
        60,
        "test cooldown",
    );

    let reset = reset_account_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "reset-me".into())),
    )
    .await
    .0;
    assert_eq!(reset, serde_json::json!({"reset": true}));
    assert_eq!(
        state
            .vault
            .get_account(&AccountEngineKind::Antigravity, "reset-me")
            .map(|account| account.status),
        Some(agent_engine::AccountStatus::Ready)
    );

    let already_ready = reset_account_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "reset-me".into())),
    )
    .await
    .0;
    assert_eq!(already_ready, serde_json::json!({"reset": true}));

    let missing = reset_account_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "missing".into())),
    )
    .await
    .0;
    assert_eq!(missing, serde_json::json!({"reset": false}));

    let unknown_engine = reset_account_handler(
        State(state),
        axum::extract::Path(("unknown-engine".into(), "reset-me".into())),
    )
    .await
    .0;
    assert_eq!(unknown_engine, serde_json::json!({"reset": false}));
    Ok(())
}

#[tokio::test]
async fn test_import_local_handler_reports_success_and_failure(
) -> Result<(), Box<dyn std::error::Error>> {
    let _env_guard = ENV_LOCK.lock().await;
    let state = create_test_state()?;
    register_test_account(&state, "antigravity", "import-me")?;

    let temp_home = std::env::temp_dir().join(format!("gateway_import_{}", uuid::Uuid::new_v4()));
    let credentials = temp_home.join(".gemini");
    std::fs::create_dir_all(&credentials)?;
    std::fs::write(credentials.join("settings.json"), "{\"token\":\"test\"}")?;

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &temp_home);
    let imported = import_local_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "import-me".into())),
    )
    .await
    .0;
    assert_eq!(imported, serde_json::json!({"imported": true}));
    let account = state
        .vault
        .get_account(&AccountEngineKind::Antigravity, "import-me")
        .ok_or("registered account missing")?;
    assert_eq!(
        std::fs::read_to_string(account.home_dir.join(".gemini/settings.json"))?,
        "{\"token\":\"test\"}"
    );

    std::fs::remove_dir_all(&credentials)?;
    let missing = import_local_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "not-present".into())),
    )
    .await
    .0;
    assert_eq!(missing, serde_json::json!({"imported": false}));
    register_test_account(&state, "antigravity", "no-local-credentials")?;
    let no_credentials = import_local_handler(
        State(state),
        axum::extract::Path(("antigravity".into(), "no-local-credentials".into())),
    )
    .await
    .0;
    assert_eq!(no_credentials, serde_json::json!({"imported": false}));
    restore_env_var("HOME", previous_home);
    std::fs::remove_dir_all(temp_home)?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn test_verify_account_handler_success_and_failure_paths(
) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = ENV_LOCK.lock().await;
    let state = create_test_state()?;
    register_test_account(&state, "antigravity", "verify-me")?;
    let runner = std::env::temp_dir().join(format!("gateway_verify_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&runner)?;
    let binary = runner.join("agy-test");
    std::fs::write(&binary, "#!/bin/sh\nprintf '  verified output  \\n'\n")?;
    let mut permissions = std::fs::metadata(&binary)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions)?;

    let previous_binary = std::env::var_os("AGY_BIN");
    std::env::set_var("AGY_BIN", &binary);
    state.vault.mark_cooldown(
        &AccountEngineKind::Antigravity,
        "verify-me",
        60,
        "pre-existing cooldown",
    );
    let verified = verify_account_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "verify-me".into())),
    )
    .await
    .0;
    assert_eq!(verified["success"], true);
    assert!(verified["latency_ms"].as_u64().is_some());
    assert_eq!(verified["output"], "verified output");
    assert_eq!(
        state
            .vault
            .get_account(&AccountEngineKind::Antigravity, "verify-me")
            .map(|account| account.status),
        Some(agent_engine::AccountStatus::Ready)
    );

    std::fs::write(&binary, "#!/bin/sh\nexit 7\n")?;
    let failed = verify_account_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "verify-me".into())),
    )
    .await
    .0;
    restore_env_var("AGY_BIN", previous_binary);
    assert_eq!(failed["success"], false);
    assert_eq!(failed["error"], "Engine exited with code 7");
    assert!(matches!(
        state
            .vault
            .get_account(&AccountEngineKind::Antigravity, "verify-me")
            .map(|account| account.status),
        Some(agent_engine::AccountStatus::Cooldown { .. })
    ));

    let missing = verify_account_handler(
        State(state.clone()),
        axum::extract::Path(("antigravity".into(), "missing".into())),
    )
    .await
    .0;
    assert_eq!(missing["success"], false);
    assert_eq!(missing["error"], "Account not found");

    register_test_account(&state, "custom", "generic")?;
    let generic = verify_account_handler(
        State(state.clone()),
        axum::extract::Path(("custom".into(), "generic".into())),
    )
    .await
    .0;
    assert_eq!(generic["success"], false);
    assert_eq!(
        generic["error"],
        "Account verification is unavailable for generic engines"
    );

    std::fs::remove_dir_all(runner)?;
    Ok(())
}
