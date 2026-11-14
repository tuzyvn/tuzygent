use super::fault::{AccountError, AccountFault};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AccountEngineKind {
    Codex,
    Claude,
    Generic(String),
}

impl std::fmt::Display for AccountEngineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountEngineKind::Codex => write!(f, "codex"),
            AccountEngineKind::Claude => write!(f, "claude"),
            AccountEngineKind::Generic(name) => write!(f, "{}", name),
        }
    }
}

impl AccountEngineKind {
    pub fn from_str_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

    pub fn validate(&self) -> Result<(), AccountError> {
        let trimmed_id = self.id.trim();
        if trimmed_id.is_empty() {
            return Err(AccountError::InvalidAccount(
                "Account ID cannot be empty".to_string(),
            ));
        }
        if self.name.trim().is_empty() {
            return Err(AccountError::InvalidAccount(
                "Account name cannot be empty".to_string(),
            ));
        }
        if trimmed_id == "."
            || trimmed_id == ".."
            || trimmed_id.contains('/')
            || trimmed_id.contains('\\')
            || trimmed_id.contains("..")
        {
            return Err(AccountError::InvalidAccount(format!(
                "Account ID '{}' contains path traversal or invalid characters",
                self.id
            )));
        }
        Ok(())
    }

    pub fn simulate_fault(&self, fault: &AccountFault) -> Result<Self, AccountError> {
        match fault {
            AccountFault::SocketTimeout => Err(AccountError::SocketTimeout),
            AccountFault::ConnectionReset => Err(AccountError::ConnectionReset),
            AccountFault::RateLimitExceeded => Err(AccountError::RateLimitExceeded(
                "429 RateLimitExceeded".to_string(),
            )),
            AccountFault::TransientIo => Err(AccountError::TransientIo(
                "Simulated transient I/O failure".to_string(),
            )),
            AccountFault::CorruptedMetadata => Err(AccountError::CorruptedMetadata(
                "Corrupted account metadata".to_string(),
            )),
            AccountFault::ServiceUnavailable => Err(AccountError::ServiceUnavailable(
                "Account service temporarily unavailable".to_string(),
            )),
        }
    }

    pub fn estimated_size_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let heap_id = self.id.len();
        let heap_name = self.name.len();
        let heap_home = self.home_dir.as_os_str().len();
        let heap_env: usize = self.extra_env.iter().map(|(k, v)| k.len() + v.len()).sum();
        base + heap_id + heap_name + heap_home + heap_env
    }
}
