use serde::{Deserialize, Serialize};

use super::defaults::default_true;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMount {
    pub source: String,
    pub target: String,
    #[serde(default = "default_true")]
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default)]
    pub allowed_env_vars: Vec<String>,
    #[serde(default)]
    pub mount_credentials: Vec<CredentialMount>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_env_vars: vec![
                "CODEX_API_KEY".to_string(),
                "CODEX_BIN".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
                "GEMINI_API_KEY".to_string(),
                "OPENAI_API_KEY".to_string(),
                "ANTIGRAVITY_API_KEY".to_string(),
                "AGY_BIN".to_string(),
            ],
            mount_credentials: vec![
                CredentialMount {
                    source: "~/.codex/auth.json".into(),
                    target: "/root/.codex/auth.json".into(),
                    readonly: true,
                },
                CredentialMount {
                    source: "~/.gemini".to_string(),
                    target: "/root/.gemini".to_string(),
                    readonly: true,
                },
                CredentialMount {
                    source: "~/.claude".to_string(),
                    target: "/root/.claude".to_string(),
                    readonly: true,
                },
            ],
        }
    }
}
