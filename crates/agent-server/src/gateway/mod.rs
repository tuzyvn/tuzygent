pub mod accounts;
pub mod chat;
pub mod dashboard;
pub mod types;

pub use accounts::*;
pub use chat::*;
pub use dashboard::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use agent_runtime::{AgentSupervisor, InterAgentBus};
    use agent_engine::AccountEngineKind;
    use axum::extract::State;
    use axum::Json;

    use crate::AppState;

    fn create_test_state() -> Result<AppState, Box<dyn std::error::Error>> {
        let bus = InterAgentBus::new(16);
        let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));
        let temp_dir = std::env::temp_dir().join(format!("gateway_test_{}", uuid::Uuid::new_v4().simple()));
        let vault = Arc::new(agent_engine::AccountVault::new_with_dir(temp_dir));
        let task_store = Arc::new(agent_runtime::SqliteTaskStore::memory()?);
        let task_service = Arc::new(agent_runtime::TaskService::new(
            task_store,
            "/tmp",
            2,
            None,
            None,
        ));

        Ok(AppState {
            supervisor,
            bus,
            workspace: "/tmp".into(),
            vault,
            task_service,
        })
    }

    #[tokio::test]
    async fn test_gateway_account_profile_sanitization() -> Result<(), Box<dyn std::error::Error>> {
        let state = create_test_state()?;

        let mut env = HashMap::new();
        env.insert("SECRET_API_KEY".to_string(), "sk-super-secret-12345".to_string());
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
        assert!(safe_profile.extra_env_keys.contains(&"SECRET_API_KEY".to_string()));
        assert!(safe_profile.extra_env_keys.contains(&"ANTHROPIC_AUTH".to_string()));

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
    async fn test_gateway_stream_account_resolution() -> Result<(), Box<dyn std::error::Error>> {
        // Model resolution logic for streaming & non-streaming
        let antigravity_res = parse_model_spec("antigravity");
        assert!(antigravity_res.is_ok());
        let (kind, override_model) = antigravity_res?;
        assert_eq!(kind, AccountEngineKind::Antigravity);
        assert_eq!(override_model, None);

        let gemini_custom = parse_model_spec("gemini/gemini-2.0-flash-exp");
        assert!(gemini_custom.is_ok());
        let (kind, override_model) = gemini_custom?;
        assert_eq!(kind, AccountEngineKind::Antigravity);
        assert_eq!(override_model.as_deref(), Some("gemini-2.0-flash-exp"));

        let codex_res = parse_model_spec("codex/o1-preview");
        assert!(codex_res.is_ok());
        let (kind, override_model) = codex_res?;
        assert_eq!(kind, AccountEngineKind::Codex);
        assert_eq!(override_model.as_deref(), Some("o1-preview"));

        let claude_res = parse_model_spec("claude/claude-3-5-haiku");
        assert!(claude_res.is_ok());
        let (kind, override_model) = claude_res?;
        assert_eq!(kind, AccountEngineKind::Claude);
        assert_eq!(override_model.as_deref(), Some("claude-3-5-haiku"));

        // Unauthorized / unrecognized model must fail with clear error
        let invalid = parse_model_spec("unknown_engine_v1");
        assert!(invalid.is_err());
        match invalid {
            Err(err) => assert!(err.contains("not recognized or permitted")),
            Ok(_) => return Err("expected parse_model_spec to fail for unknown model".into()),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_gateway_models_list_endpoint() -> Result<(), Box<dyn std::error::Error>> {
        let models_json = list_models_handler().await.0;
        assert_eq!(models_json["object"], "list");
        let data = models_json["data"]
            .as_array()
            .ok_or("expected data to be an array")?;
        assert!(data.iter().any(|m| m["id"] == "antigravity"));
        assert!(data.iter().any(|m| m["id"] == "codex"));
        assert!(data.iter().any(|m| m["id"] == "claude"));
        Ok(())
    }

    #[tokio::test]
    async fn test_gateway_safe_error_handling_no_panics() -> Result<(), Box<dyn std::error::Error>> {
        // Verify that invalid model spec returns descriptive error result instead of panicking
        let err_res = parse_model_spec("non_existent_model_xyz");
        assert!(err_res.is_err());
        let err_msg = err_res.err().ok_or("expected error message")?;
        assert!(err_msg.contains("not recognized or permitted"));

        // Verify that invalid account removal gracefully returns an Err instead of panicking
        let state = create_test_state()?;
        let remove_res = remove_account_handler(
            State(state),
            axum::extract::Path(("antigravity".to_string(), "non-existent-acc-id".to_string())),
        ).await;
        assert!(remove_res.is_err());
        let (status, reason) = remove_res.err().ok_or("expected error status and reason")?;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(reason, "Account not found");

        Ok(())
    }
}
