use crate::sandbox::command;
pub use crate::sandbox::command::expand_home;
use crate::sandbox::config::{IsolationMode, SandboxConfig};
use crate::sandbox::context;
use crate::sandbox::workspace::WorkspaceManager;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Thin coordinator orchestrating workspace isolation, sidecar context, and command sandboxing.
pub struct SandboxEnvironment {
    pub base_repo_path: PathBuf,
    pub config: SandboxConfig,
    pub worktree_path: Option<PathBuf>,
    pub branch_name: Option<String>,
}

impl SandboxEnvironment {
    pub fn new<P: AsRef<Path>>(base_repo_path: P, mode: IsolationMode) -> Self {
        let config = SandboxConfig {
            mode,
            ..Default::default()
        };
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

    /// Khởi tạo workspace sandbox cho task (delegates to WorkspaceManager)
    pub async fn setup_workspace(&mut self, task_id: &str) -> Result<PathBuf> {
        self.prepare(task_id).await
    }

    /// Khởi tạo môi trường sandbox cô lập theo kiến trúc Multica (delegates to WorkspaceManager)
    pub async fn prepare(&mut self, task_id: &str) -> Result<PathBuf> {
        let mut manager = WorkspaceManager {
            base_repo_path: self.base_repo_path.clone(),
            mode: self.config.mode,
            worktree_path: self.worktree_path.clone(),
            branch_name: self.branch_name.clone(),
        };
        let path = manager.prepare(task_id).await?;
        self.worktree_path = manager.worktree_path;
        self.branch_name = manager.branch_name;
        Ok(path)
    }

    /// Wrap an OS command with the configured Sandbox driver (delegates to command::wrap_command)
    pub fn wrap_command(
        &self,
        original_bin: &str,
        original_args: &[String],
        run_dir: &Path,
    ) -> (String, Vec<String>) {
        command::wrap_command(&self.config, original_bin, original_args, run_dir)
    }

    /// Generates execution sandbox parameters for process engine (delegates to command::command_sandbox)
    pub fn command_sandbox(&self, run_dir: &Path) -> Option<agent_engine::process::CommandSandbox> {
        command::command_sandbox(&self.config, run_dir)
    }

    /// Tạo file Briefing Context (.multica / .agent_context) trước khi Agent chạy (delegates to sandbox::context)
    pub async fn inject_agent_brief(&self, target_dir: &Path, brief_content: &str) -> Result<()> {
        context::inject_agent_brief(target_dir, brief_content).await
    }

    /// Theo chuẩn Multica: Bắt buộc tẩy sạch file brief/context và sidecar trước khi commit (delegates to sandbox::context)
    pub async fn purge_sidecar_context(&self, target_dir: &Path) -> Result<()> {
        context::purge_sidecar_context(target_dir).await
    }

    /// Dọn dẹp worktree sau khi hoàn tất lượt chạy (delegates to WorkspaceManager)
    pub async fn teardown(&mut self) -> Result<()> {
        let mut manager = WorkspaceManager {
            base_repo_path: self.base_repo_path.clone(),
            mode: self.config.mode,
            worktree_path: self.worktree_path.clone(),
            branch_name: self.branch_name.clone(),
        };
        let res = manager.teardown().await;
        self.worktree_path = manager.worktree_path;
        self.branch_name = manager.branch_name;
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_coordinator_delegation_lifecycle() -> Result<()> {
        let temp_base =
            std::env::temp_dir().join(format!("test_coord_{}", Uuid::new_v4().simple()));
        let mut env = SandboxEnvironment::new(&temp_base, IsolationMode::Ephemeral);
        let run_dir = env.prepare("task-coord").await?;
        assert!(run_dir.exists());
        assert_eq!(env.worktree_path.as_ref(), Some(&run_dir));

        let (bin, _) = env.wrap_command("ls", &[], &run_dir);
        assert!(!bin.is_empty());

        let brief = "Coordinator briefing";
        env.inject_agent_brief(&run_dir, brief).await?;
        assert!(run_dir.join(".multica/brief.md").exists());
        env.purge_sidecar_context(&run_dir).await?;
        assert!(!run_dir.join(".multica/brief.md").exists());

        env.teardown().await?;
        assert!(!run_dir.exists());
        assert!(env.worktree_path.is_none());
        Ok(())
    }
}
