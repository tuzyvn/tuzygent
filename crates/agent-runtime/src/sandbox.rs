use agent_engine::config::{CredentialMount, SecurityConfig};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationMode {
    LocalDirectory,
    Worktree,
    Ephemeral,
}

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
        // 1. Check Docker daemon responsiveness
        if let Ok(output) = std::process::Command::new("docker").arg("info").output() {
            if output.status.success() {
                info!("🐳 Docker daemon detected and responsive.");
                return SandboxDriverKind::Docker;
            }
        }

        // 2. Check Bubblewrap
        if let Ok(output) = std::process::Command::new("bwrap")
            .arg("--version")
            .output()
        {
            if output.status.success() {
                info!("🛡️ Bubblewrap (bwrap) detected for rootless Linux sandboxing.");
                return SandboxDriverKind::Bubblewrap;
            }
        }

        warn!("⚠️ No container or namespace sandbox available; falling back to Host driver.");
        SandboxDriverKind::Host
    }
}

pub struct SandboxEnvironment {
    pub base_repo_path: PathBuf,
    pub config: SandboxConfig,
    pub worktree_path: Option<PathBuf>,
    pub branch_name: Option<String>,
}

impl SandboxEnvironment {
    pub fn new<P: AsRef<Path>>(base_repo_path: P, mode: IsolationMode) -> Self {
        let mut config = SandboxConfig::default();
        config.mode = mode;
        Self {
            base_repo_path: base_repo_path.as_ref().to_path_buf(),
            config,
            worktree_path: None,
            branch_name: None,
        }
    }

    pub fn with_config<P: AsRef<Path>>(base_repo_path: P, config: SandboxConfig) -> Self {
        Self {
            base_repo_path: base_repo_path.as_ref().to_path_buf(),
            config,
            worktree_path: None,
            branch_name: None,
        }
    }

    /// Khởi tạo môi trường sandbox cô lập theo kiến trúc Multica
    pub async fn prepare(&mut self, task_id: &str) -> Result<PathBuf> {
        match self.config.mode {
            IsolationMode::LocalDirectory => {
                info!(
                    "📂 Using local directory sandbox: {:?}",
                    self.base_repo_path
                );
                Ok(self.base_repo_path.clone())
            }
            IsolationMode::Worktree => {
                let branch = format!("agent/task-{}", task_id.to_lowercase().replace('_', "-"));
                let worktree_dir = self
                    .base_repo_path
                    .parent()
                    .unwrap_or(&self.base_repo_path)
                    .join(format!(".worktrees/{}", Uuid::new_v4().simple()));

                info!(
                    "🌿 Creating Git Worktree at {:?} on branch {}",
                    worktree_dir, branch
                );

                let status = Command::new("git")
                    .current_dir(&self.base_repo_path)
                    .args(["worktree", "add", "-b", &branch])
                    .arg(&worktree_dir)
                    .arg("HEAD")
                    .status()
                    .await
                    .context("Failed to execute git worktree add")?;

                if !status.success() {
                    anyhow::bail!(
                        "git worktree add exited with error code: {:?}",
                        status.code()
                    );
                }

                self.worktree_path = Some(worktree_dir.clone());
                self.branch_name = Some(branch);

                Ok(worktree_dir)
            }
            IsolationMode::Ephemeral => {
                let temp_dir =
                    std::env::temp_dir().join(format!("agent-sandbox-{}", Uuid::new_v4().simple()));
                fs::create_dir_all(&temp_dir).await?;
                self.worktree_path = Some(temp_dir.clone());
                Ok(temp_dir)
            }
        }
    }

    /// Wrap an OS command with the configured Sandbox driver (Docker or Bubblewrap)
    pub fn wrap_command(
        &self,
        original_bin: &str,
        original_args: &[String],
        run_dir: &Path,
    ) -> (String, Vec<String>) {
        let resolved_dir = std::fs::canonicalize(run_dir).unwrap_or_else(|_| run_dir.to_path_buf());
        let run_dir = resolved_dir.as_path();
        match self.config.driver {
            SandboxDriverKind::Bubblewrap => {
                let mut bwrap_args = vec![
                    "--unshare-pid".to_string(),
                    "--unshare-ipc".to_string(),
                    "--unshare-uts".to_string(),
                    "--die-with-parent".to_string(),
                    "--new-session".to_string(),
                    "--dev".to_string(),
                    "/dev".to_string(),
                    "--proc".to_string(),
                    "/proc".to_string(),
                    "--tmpfs".to_string(),
                    "/tmp".to_string(),
                ];

                // Expose runtimes and public system configuration, not the host home/root.
                for path in [
                    "/usr",
                    "/bin",
                    "/sbin",
                    "/lib",
                    "/lib64",
                    "/etc/ssl",
                    "/etc/ca-certificates",
                    "/etc/pki",
                    "/etc/nsswitch.conf",
                    "/etc/gai.conf",
                    "/etc/host.conf",
                    "/etc/resolv.conf",
                    "/etc/hosts",
                    "/etc/passwd",
                    "/etc/group",
                ] {
                    if Path::new(path).exists() {
                        bwrap_args.extend(["--ro-bind".into(), path.into(), path.into()]);
                    }
                }
                bwrap_args.extend([
                    "--dir".into(),
                    "/root".into(),
                    "--setenv".into(),
                    "HOME".into(),
                    "/root".into(),
                ]);

                // Mount target working directory read-write
                let dir_str = run_dir.to_string_lossy().to_string();
                bwrap_args.push("--bind".to_string());
                bwrap_args.push(dir_str.clone());
                bwrap_args.push(dir_str.clone());

                bwrap_args.push("--chdir".to_string());
                bwrap_args.push(dir_str);

                // Mount read-only credentials if they exist
                for mount in &self.config.ro_credentials {
                    let expanded_source = expand_home(&mount.source);
                    if expanded_source.exists() {
                        bwrap_args.push("--ro-bind".to_string());
                        bwrap_args.push(expanded_source.to_string_lossy().to_string());
                        bwrap_args.push(mount.target.clone());
                    }
                }

                // Inject allowed environment variables
                for env_key in &self.config.env_whitelist {
                    if let Ok(val) = std::env::var(env_key) {
                        bwrap_args.push("--setenv".to_string());
                        bwrap_args.push(env_key.clone());
                        bwrap_args.push(val);
                    }
                }

                // Append original command and arguments
                bwrap_args.push(original_bin.to_string());
                bwrap_args.extend_from_slice(original_args);

                ("bwrap".to_string(), bwrap_args)
            }

            SandboxDriverKind::Docker => {
                let mut docker_args = vec![
                    "run".to_string(),
                    "--rm".to_string(),
                    "-i".to_string(),
                    "--cap-drop=ALL".to_string(),
                    "--security-opt=no-new-privileges".to_string(),
                    "--pids-limit=256".to_string(),
                    "-v".to_string(),
                    format!("{}:/workspace", run_dir.to_string_lossy()),
                    "-w".to_string(),
                    "/workspace".to_string(),
                ];

                if let Some(mem) = self.config.memory_limit_mb {
                    docker_args.push("--memory".to_string());
                    docker_args.push(format!("{}m", mem));
                }

                for mount in &self.config.ro_credentials {
                    let expanded_source = expand_home(&mount.source);
                    if expanded_source.exists() {
                        docker_args.push("-v".to_string());
                        docker_args.push(format!(
                            "{}:{}:ro",
                            expanded_source.to_string_lossy(),
                            mount.target
                        ));
                    }
                }

                for env_key in &self.config.env_whitelist {
                    if let Ok(val) = std::env::var(env_key) {
                        docker_args.push("-e".to_string());
                        docker_args.push(format!("{}={}", env_key, val));
                    }
                }

                docker_args.push(self.config.docker_image.clone());
                docker_args.push(original_bin.to_string());
                docker_args.extend_from_slice(original_args);

                ("docker".to_string(), docker_args)
            }

            SandboxDriverKind::Host => (original_bin.to_string(), original_args.to_vec()),
        }
    }

    pub fn command_sandbox(&self, run_dir: &Path) -> Option<agent_engine::process::CommandSandbox> {
        if self.config.driver == SandboxDriverKind::Host {
            return None;
        }
        let (program, mut prefix_args) = self.wrap_command("__engine__", &[], run_dir);
        prefix_args.pop();
        let container_name = if self.config.driver == SandboxDriverKind::Docker {
            let name = format!("tuzygent-{}", Uuid::new_v4().simple());
            prefix_args.splice(1..1, ["--name".into(), name.clone()]);
            Some(name)
        } else {
            None
        };
        Some(agent_engine::process::CommandSandbox {
            program,
            prefix_args,
            container_name,
            env_whitelist: self.config.env_whitelist.clone(),
            working_dir: if self.config.driver == SandboxDriverKind::Docker {
                "/workspace".into()
            } else {
                std::fs::canonicalize(run_dir)
                    .unwrap_or_else(|_| run_dir.to_path_buf())
                    .to_string_lossy()
                    .into_owned()
            },
        })
    }

    /// Tạo file Briefing Context (.multica / .agent_context) trước khi Agent chạy
    pub async fn inject_agent_brief(&self, target_dir: &Path, brief_content: &str) -> Result<()> {
        let meta_dir = target_dir.join(".multica");
        fs::create_dir_all(&meta_dir).await?;
        fs::write(meta_dir.join("brief.md"), brief_content).await?;
        fs::write(target_dir.join(".agent_context.md"), brief_content).await?;
        Ok(())
    }

    /// Theo chuẩn Multica: Bắt buộc tẩy sạch file brief/context và sidecar trước khi commit
    pub async fn purge_sidecar_context(&self, target_dir: &Path) -> Result<()> {
        let agent_ctx = target_dir.join(".agent_context.md");
        if agent_ctx.exists() {
            let _ = fs::remove_file(agent_ctx).await;
        }

        let meta_dir = target_dir.join(".multica");
        if meta_dir.exists() {
            let _ = fs::remove_dir_all(meta_dir).await;
        }

        Ok(())
    }

    /// Dọn dẹp worktree sau khi hoàn tất lượt chạy
    pub async fn teardown(&mut self) -> Result<()> {
        if let Some(worktree_dir) = &self.worktree_path {
            if self.config.mode == IsolationMode::Worktree {
                info!("🧹 Cleaning up Git Worktree at {:?}", worktree_dir);
                let _ = Command::new("git")
                    .current_dir(&self.base_repo_path)
                    .args(["worktree", "remove", "--force"])
                    .arg(worktree_dir)
                    .status()
                    .await;
            } else if self.config.mode == IsolationMode::Ephemeral {
                let _ = fs::remove_dir_all(worktree_dir).await;
            }
            self.worktree_path = None;
        }
        Ok(())
    }
}

fn expand_home(path_str: &str) -> PathBuf {
    if path_str.starts_with("~/") || path_str == "~" {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return PathBuf::from(path_str.replacen('~', &home, 1));
        }
    }
    PathBuf::from(path_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_detection() {
        let driver = SandboxConfig::detect_best_driver();
        // On this linux machine, bubblewrap is installed and should be detected
        assert!(
            driver == SandboxDriverKind::Bubblewrap
                || driver == SandboxDriverKind::Docker
                || driver == SandboxDriverKind::Host
        );
    }

    #[test]
    fn test_bwrap_wrap_command() {
        let mut config = SandboxConfig::default();
        config.driver = SandboxDriverKind::Bubblewrap;
        let env = SandboxEnvironment::with_config(".", config);

        let (bin, args) = env.wrap_command("ls", &["-la".to_string()], Path::new("/tmp"));
        assert_eq!(bin, "bwrap");
        assert!(args.contains(&"--ro-bind".to_string()));
        assert!(args.contains(&"ls".to_string()));
        assert!(args.contains(&"-la".to_string()));
    }
    #[test]
    fn bwrap_includes_public_certificate_stores() {
        let environment = SandboxEnvironment::with_config(
            ".",
            SandboxConfig {
                driver: SandboxDriverKind::Bubblewrap,
                ..Default::default()
            },
        );
        let (_, args) = environment.wrap_command("true", &[], Path::new("/tmp"));
        for store in ["/etc/ssl", "/etc/ca-certificates", "/etc/pki"] {
            if Path::new(store).exists() {
                assert!(
                    args.windows(3)
                        .any(|args| args == ["--ro-bind", store, store]),
                    "missing CA store {store}"
                );
            }
        }
        assert!(!args.windows(3).any(|args| args == ["--ro-bind", "/", "/"]));
    }
}
