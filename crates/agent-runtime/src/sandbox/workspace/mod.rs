pub mod benchmark;
pub mod fault;
pub mod manager;
pub mod resilience;

pub use benchmark::{WorkspaceBenchmarkHarness, WorkspaceBenchmarkStats};
pub use fault::{
    is_retryable_workspace_error, WorkspaceError, WorkspaceFaultKind, WorkspaceFaultPolicy,
};
pub use manager::{prepare_workspace, teardown_workspace, WorkspaceManager};
pub use resilience::{ResilientWorkspaceManager, WorkspaceAuditRecord};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::config::IsolationMode;
    use anyhow::Result;
    use uuid::Uuid;

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
