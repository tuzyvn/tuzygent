use super::driver::SandboxDriverKind;
use agent_engine::config::{CredentialMount, SecurityConfig};
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationMode {
    LocalDirectory,
    Worktree,
    Ephemeral,
}

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub mode: IsolationMode,
    pub driver: SandboxDriverKind,
    pub docker_image: String,
    pub ro_credentials: Vec<CredentialMount>,
    pub env_whitelist: Vec<String>,
    pub memory_limit_mb: Option<u64>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        let sec = SecurityConfig::default();
        Self {
            mode: IsolationMode::LocalDirectory,
            driver: if cfg!(target_os = "linux") {
                SandboxDriverKind::Bubblewrap
            } else {
                SandboxDriverKind::Docker
            },
            docker_image: "tuzygent-engine:latest".to_string(),
            ro_credentials: sec.mount_credentials,
            env_whitelist: sec.allowed_env_vars,
            memory_limit_mb: Some(4096),
        }
    }
}

impl SandboxConfig {
    pub fn from_engines_config(config: &agent_engine::EnginesConfig) -> Result<Self> {
        Ok(Self {
            mode: IsolationMode::LocalDirectory,
            driver: SandboxDriverKind::parse(&config.defaults.sandbox_driver)?,
            docker_image: config.defaults.sandbox_image.clone(),
            ro_credentials: config.security.mount_credentials.clone(),
            env_whitelist: config.security.allowed_env_vars.clone(),
            memory_limit_mb: Some(4096),
        })
    }

    /// Detect best available sandbox driver:
    /// 1. Docker (if daemon is responding)
    /// 2. Bubblewrap (bwrap on Linux)
    /// 3. Host process fallback
    pub fn detect_best_driver() -> SandboxDriverKind {
        SandboxDriverKind::detect_best_driver()
    }
}
