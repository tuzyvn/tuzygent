use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AccountEngineKind {
    Antigravity,
    Codex,
    Claude,
    Generic(String),
}

impl std::fmt::Display for AccountEngineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountEngineKind::Antigravity => write!(f, "antigravity"),
            AccountEngineKind::Codex => write!(f, "codex"),
            AccountEngineKind::Claude => write!(f, "claude"),
            AccountEngineKind::Generic(name) => write!(f, "{}", name),
        }
    }
}

impl AccountEngineKind {
    pub fn from_str_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "antigravity" | "agy" => AccountEngineKind::Antigravity,
            "codex" => AccountEngineKind::Codex,
            "claude" | "claude_code" | "claude-code" => AccountEngineKind::Claude,
            other => AccountEngineKind::Generic(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccountStatus {
    Ready,
    Cooldown {
        until: DateTime<Utc>,
        reason: String,
    },
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProfile {
    pub id: String,
    pub name: String,
    pub engine: AccountEngineKind,
    pub home_dir: PathBuf,
    pub status: AccountStatus,
    pub requests_count: u64,
    pub failure_count: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub extra_env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeAccountProfile {
    pub id: String,
    pub name: String,
    pub engine: AccountEngineKind,
    pub home_dir: PathBuf,
    pub status: AccountStatus,
    pub requests_count: u64,
    pub failure_count: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub extra_env_keys: Vec<String>,
}

impl AccountProfile {
    pub fn is_available(&self) -> bool {
        match &self.status {
            AccountStatus::Ready => true,
            AccountStatus::Cooldown { until, .. } => Utc::now() >= *until,
            AccountStatus::Disabled => false,
        }
    }

    pub fn to_safe(&self) -> SafeAccountProfile {
        SafeAccountProfile {
            id: self.id.clone(),
            name: self.name.clone(),
            engine: self.engine.clone(),
            home_dir: self.home_dir.clone(),
            status: self.status.clone(),
            requests_count: self.requests_count,
            failure_count: self.failure_count,
            last_used_at: self.last_used_at,
            extra_env_keys: self.extra_env.keys().cloned().collect(),
        }
    }
}
