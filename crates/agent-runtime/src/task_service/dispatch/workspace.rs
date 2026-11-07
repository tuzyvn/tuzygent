use super::super::TaskService;
use std::path::{Path, PathBuf};

impl TaskService {
    /// Safe multi-tier fallback workspace directory resolution
    pub fn resolve_workspace(workspace: &Path, task_id_str: &str) -> PathBuf {
        let trimmed = workspace.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() && (workspace.exists() || std::fs::create_dir_all(workspace).is_ok())
        {
            return workspace.to_path_buf();
        }
        let unique = format!("agent_ws_{}_{}", task_id_str, uuid::Uuid::new_v4());
        let candidates = [
            std::env::temp_dir().join(&unique),
            PathBuf::from("target/tmp").join(&unique),
            PathBuf::from("/tmp").join(&unique),
        ];
        for candidate in candidates {
            if std::fs::create_dir_all(&candidate).is_ok() {
                return candidate;
            }
        }
        std::env::temp_dir()
    }
}
