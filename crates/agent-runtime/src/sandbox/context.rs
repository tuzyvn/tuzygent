use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Quản lý vòng đời tệp tin brief và sidecar context của agent.
#[derive(Debug, Clone)]
pub struct SidecarContextManager {
    pub target_dir: PathBuf,
}

impl SidecarContextManager {
    pub fn new<P: AsRef<Path>>(target_dir: P) -> Self {
        Self {
            target_dir: target_dir.as_ref().to_path_buf(),
        }
    }

    /// Tạo file Briefing Context (.multica / .agent_context) trước khi Agent chạy
    pub async fn inject_agent_brief(&self, brief_content: &str) -> Result<()> {
        inject_agent_brief(&self.target_dir, brief_content).await
    }

    /// Theo chuẩn Multica: Bắt buộc tẩy sạch file brief/context và sidecar trước khi commit
    pub async fn purge_sidecar_context(&self) -> Result<()> {
        purge_sidecar_context(&self.target_dir).await
    }
}

/// Tạo file Briefing Context (.multica / .agent_context) trước khi Agent chạy
pub async fn inject_agent_brief(target_dir: &Path, brief_content: &str) -> Result<()> {
    let meta_dir = target_dir.join(".multica");
    fs::create_dir_all(&meta_dir).await?;
    fs::write(meta_dir.join("brief.md"), brief_content).await?;
    fs::write(target_dir.join(".agent_context.md"), brief_content).await?;
    Ok(())
}

/// Theo chuẩn Multica: Bắt buộc tẩy sạch file brief/context và sidecar trước khi commit
pub async fn purge_sidecar_context(target_dir: &Path) -> Result<()> {
    let agent_ctx = target_dir.join(".agent_context.md");
    if agent_ctx.exists() {
        let _ = fs::remove_file(agent_ctx).await;
    }

    // Only remove task-generated briefing and runtime context files, do NOT delete the entire .multica directory
    let meta_dir = target_dir.join(".multica");
    if meta_dir.exists() {
        let brief_file = meta_dir.join("brief.md");
        if brief_file.exists() {
            let _ = fs::remove_file(brief_file).await;
        }
        let exec_file = meta_dir.join("execution_context.md");
        if exec_file.exists() {
            let _ = fs::remove_file(exec_file).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_sidecar_context_manager_lifecycle() -> Result<()> {
        let temp_dir =
            std::env::temp_dir().join(format!("ctx_mgr_test_{}", Uuid::new_v4().simple()));
        let _ = fs::create_dir_all(&temp_dir).await;
        let manager = SidecarContextManager::new(&temp_dir);

        let brief = "Manager Briefing Content";
        manager.inject_agent_brief(brief).await?;

        assert_eq!(
            fs::read_to_string(temp_dir.join(".agent_context.md")).await?,
            brief
        );
        assert_eq!(
            fs::read_to_string(temp_dir.join(".multica/brief.md")).await?,
            brief
        );

        manager.purge_sidecar_context().await?;
        assert!(!temp_dir.join(".agent_context.md").exists());
        assert!(!temp_dir.join(".multica/brief.md").exists());

        let _ = fs::remove_dir_all(&temp_dir).await;
        Ok(())
    }
}
