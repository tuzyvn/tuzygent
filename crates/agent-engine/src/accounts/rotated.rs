use async_trait::async_trait;
use std::sync::Arc;

use super::types::AccountEngineKind;
use super::vault::AccountVault;
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

/// ComputeEngine tự động xoay vòng tài khoản CLI (Codex, Claude...)
/// theo danh sách tài khoản trong AccountVault
pub struct AccountRotatedEngine {
    vault: Arc<AccountVault>,
    engine_kind: AccountEngineKind,
    binary_path: Option<String>,
    model: Option<String>,
}

impl AccountRotatedEngine {
    pub fn new(
        vault: Arc<AccountVault>,
        engine_kind: AccountEngineKind,
        binary_path: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            vault,
            engine_kind,
            binary_path,
            model,
        }
    }
}

#[async_trait]
impl ComputeEngine for AccountRotatedEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        if let AccountEngineKind::Generic(name) = &self.engine_kind {
            if matches!(
                name.trim().to_ascii_lowercase().as_str(),
                "antigravity" | "agy" | "gemini"
            ) {
                return Err(EngineError::PolicyRejected(
                    "Multi-account rotation for Google/Antigravity is disabled to comply with Google Terms of Service".to_string(),
                ));
            }
        }

        let max_retries = 3;
        let mut last_err = None;

        let accounts = self.vault.list_accounts(Some(&self.engine_kind));
        if accounts.is_empty() {
            return Err(EngineError::ProcessFailed(format!(
                "No account configured for engine '{}'. Please register an account first.",
                self.engine_kind
            )));
        }

        for _ in 0..max_retries {
            let account = match self.vault.select_next_account(&self.engine_kind) {
                Some(acc) => acc,
                None => {
                    return Err(EngineError::ProcessFailed(format!(
                        "All accounts for engine '{}' are currently in cooldown or unavailable.",
                        self.engine_kind
                    )));
                }
            };
            let env = self.vault.build_env_for_account(&account);

            let engine: Arc<dyn ComputeEngine> = match &self.engine_kind {
                AccountEngineKind::Codex => Arc::new(
                    crate::cli::CodexEngine::new(self.binary_path.clone(), self.model.clone())
                        .with_env(env),
                ),
                AccountEngineKind::Claude => Arc::new(
                    crate::cli::ClaudeCodeEngine::new(self.binary_path.clone(), self.model.clone())
                        .with_env(env),
                ),
                AccountEngineKind::Generic(name) => {
                    if name.contains('/') || name.contains('\\') || name.contains("..") {
                        return Err(EngineError::ProcessFailed(format!(
                            "Invalid generic engine name '{}'",
                            name
                        )));
                    }
                    Arc::new(crate::cli::GenericCliEngine::new(
                        name.clone(),
                        self.binary_path.clone().unwrap_or_else(|| name.clone()),
                        vec![],
                        env,
                        None,
                    ))
                }
            };

            match engine.execute(input).await {
                Ok(output) => {
                    self.vault.record_success(&self.engine_kind, &account.id);
                    return Ok(output);
                }
                Err(err) => {
                    let cooldown_secs = if err.is_auth_or_quota() { 300 } else { 60 };
                    self.vault.mark_cooldown(
                        &self.engine_kind,
                        &account.id,
                        cooldown_secs,
                        &err.to_string(),
                    );
                    last_err = Some(err);
                    if self.vault.list_accounts(Some(&self.engine_kind)).len() <= 1 {
                        break;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            EngineError::ProcessFailed("Account pool exhausted or execution failed".to_string())
        }))
    }

    fn engine_kind(&self) -> EngineKind {
        match &self.engine_kind {
            AccountEngineKind::Codex => EngineKind::Codex,
            AccountEngineKind::Claude => EngineKind::ClaudeCode,
            AccountEngineKind::Generic(n) => EngineKind::Generic(n.clone()),
        }
    }
}
