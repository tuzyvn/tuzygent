use anyhow::Result;
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxDriverKind {
    Docker,
    Bubblewrap,
    Host,
}

impl SandboxDriverKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_lowercase().as_str() {
            "docker" => Ok(Self::Docker),
            "bubblewrap" | "bwrap" => Ok(Self::Bubblewrap),
            "host" => Ok(Self::Host),
            _ => anyhow::bail!("Unknown sandbox driver: {value}"),
        }
    }

    /// Detect best available sandbox driver:
    /// 1. Docker (if daemon is responding)
    /// 2. Bubblewrap (bwrap on Linux)
    /// 3. Host process fallback
    pub fn detect_best_driver() -> Self {
        // 1. Check Docker daemon responsiveness
        if let Ok(output) = std::process::Command::new("docker").arg("info").output() {
            if output.status.success() {
                info!("🐳 Docker daemon detected and responsive.");
                return Self::Docker;
            }
        }

        // 2. Check Bubblewrap
        if let Ok(output) = std::process::Command::new("bwrap")
            .arg("--version")
            .output()
        {
            if output.status.success() {
                info!("🛡️ Bubblewrap (bwrap) detected for rootless Linux sandboxing.");
                return Self::Bubblewrap;
            }
        }

        warn!("⚠️ No container or namespace sandbox available; falling back to Host driver.");
        Self::Host
    }
}

impl std::fmt::Display for SandboxDriverKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxDriverKind::Docker => write!(f, "docker"),
            SandboxDriverKind::Bubblewrap => write!(f, "bubblewrap"),
            SandboxDriverKind::Host => write!(f, "host"),
        }
    }
}
