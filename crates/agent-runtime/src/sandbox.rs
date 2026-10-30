use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;
use uuid::Uuid;
use anyhow::{Context, Result};
use tracing::info;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationMode {
    LocalDirectory,
    Worktree,
    Ephemeral,
}

pub struct SandboxEnvironment {
    pub base_repo_path: PathBuf,
    pub mode: IsolationMode,
    pub worktree_path: Option<PathBuf>,
    pub branch_name: Option<String>,
}

impl SandboxEnvironment {
    pub fn new<P: AsRef<Path>>(base_repo_path: P, mode: IsolationMode) -> Self {
        Self {
            base_repo_path: base_repo_path.as_ref().to_path_buf(),
            mode,
            worktree_path: None,
            branch_name: None,
        }
    }

    /// Khởi tạo môi trường sandbox cô lập theo kiến trúc Multica
    pub async fn prepare(&mut self, task_id: &str) -> Result<PathBuf> {
        match self.mode {
            IsolationMode::LocalDirectory => {
                info!("📂 Using local directory sandbox: {:?}", self.base_repo_path);
                Ok(self.base_repo_path.clone())
            }
            IsolationMode::Worktree => {
                let branch = format!("agent/task-{}", task_id.to_lowercase().replace('_', "-"));
                let worktree_dir = self
                    .base_repo_path
                    .parent()
                    .unwrap_or(&self.base_repo_path)
                    .join(format!(".worktrees/{}", Uuid::new_v4().simple()));

                info!("🌿 Creating Git Worktree at {:?} on branch {}", worktree_dir, branch);

                // git worktree add -b <branch> <path> HEAD
                let status = Command::new("git")
                    .current_dir(&self.base_repo_path)
                    .args(["worktree", "add", "-b", &branch])
                    .arg(&worktree_dir)
                    .arg("HEAD")
                    .status()
                    .await
                    .context("Failed to execute git worktree add")?;

                if !status.success() {
                    anyhow::bail!("git worktree add exited with error code: {:?}", status.code());
                }

                self.worktree_path = Some(worktree_dir.clone());
                self.branch_name = Some(branch);

                Ok(worktree_dir)
            }
            IsolationMode::Ephemeral => {
                let temp_dir = std::env::temp_dir().join(format!("agent-sandbox-{}", Uuid::new_v4().simple()));
                fs::create_dir_all(&temp_dir).await?;
                self.worktree_path = Some(temp_dir.clone());
                Ok(temp_dir)
            }
        }
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
