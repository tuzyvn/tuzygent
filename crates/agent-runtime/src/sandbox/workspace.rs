use crate::sandbox::config::IsolationMode;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;
use tracing::info;
use uuid::Uuid;

/// Quản lý vòng đời workspace sandbox cô lập (LocalDirectory, Ephemeral, Git Worktree).
#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    pub base_repo_path: PathBuf,
    pub mode: IsolationMode,
    pub worktree_path: Option<PathBuf>,
    pub branch_name: Option<String>,
}

impl WorkspaceManager {
    pub fn new<P: AsRef<Path>>(base_repo_path: P, mode: IsolationMode) -> Self {
        Self {
            base_repo_path: base_repo_path.as_ref().to_path_buf(),
            mode,
            worktree_path: None,
            branch_name: None,
        }
    }

    /// Khởi tạo workspace sandbox cho task
    pub async fn setup_workspace(&mut self, task_id: &str) -> Result<PathBuf> {
        self.prepare(task_id).await
    }

    /// Khởi tạo môi trường sandbox cô lập theo kiến trúc Multica
    pub async fn prepare(&mut self, task_id: &str) -> Result<PathBuf> {
        match self.mode {
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

    /// Dọn dẹp worktree sau khi hoàn tất lượt chạy
    pub async fn teardown(&mut self) -> Result<()> {
        if let Some(worktree_dir) = &self.worktree_path {
            if self.mode == IsolationMode::Worktree {
                info!("🧹 Cleaning up Git Worktree at {:?}", worktree_dir);
                let _ = Command::new("git")
                    .current_dir(&self.base_repo_path)
                    .args(["worktree", "remove", "--force"])
                    .arg(worktree_dir)
                    .status()
                    .await;
            } else if self.mode == IsolationMode::Ephemeral {
                let _ = fs::remove_dir_all(worktree_dir).await;
            }
            self.worktree_path = None;
        }
        Ok(())
    }
}

/// Helper function to prepare workspace directly
pub async fn prepare_workspace(
    base_repo_path: &Path,
    mode: IsolationMode,
    task_id: &str,
) -> Result<(PathBuf, Option<PathBuf>, Option<String>)> {
    let mut manager = WorkspaceManager::new(base_repo_path, mode);
    let run_dir = manager.prepare(task_id).await?;
    Ok((run_dir, manager.worktree_path, manager.branch_name))
}

/// Helper function to teardown workspace directly
pub async fn teardown_workspace(
    base_repo_path: &Path,
    mode: IsolationMode,
    worktree_path: &mut Option<PathBuf>,
) -> Result<()> {
    let mut manager = WorkspaceManager {
        base_repo_path: base_repo_path.to_path_buf(),
        mode,
        worktree_path: worktree_path.clone(),
        branch_name: None,
    };
    let res = manager.teardown().await;
    *worktree_path = manager.worktree_path;
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_workspace_manager_local() -> Result<()> {
        let temp_dir = std::env::temp_dir().join(format!("ws_local_{}", Uuid::new_v4().simple()));
        let mut manager = WorkspaceManager::new(&temp_dir, IsolationMode::LocalDirectory);
        let path = manager.setup_workspace("task-1").await?;
        assert_eq!(path, temp_dir);
        assert!(manager.worktree_path.is_none());
        manager.teardown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_workspace_manager_ephemeral() -> Result<()> {
        let temp_dir = std::env::temp_dir().join(format!("ws_eph_{}", Uuid::new_v4().simple()));
        let mut manager = WorkspaceManager::new(&temp_dir, IsolationMode::Ephemeral);
        let path = manager.prepare("task-eph").await?;
        assert!(path.exists());
        assert_eq!(manager.worktree_path.as_ref(), Some(&path));

        manager.teardown().await?;
        assert!(!path.exists());
        assert!(manager.worktree_path.is_none());
        Ok(())
    }
}
