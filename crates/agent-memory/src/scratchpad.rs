use std::path::{Path, PathBuf};
use tokio::fs;

pub struct ScratchpadManager {
    file_path: PathBuf,
}

impl ScratchpadManager {
    pub fn new<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self {
            file_path: workspace_dir.as_ref().join("memory").join("scratchpad.md"),
        }
    }

    pub async fn read(&self) -> Result<String, std::io::Error> {
        if !self.file_path.exists() {
            return Ok(String::new());
        }
        fs::read_to_string(&self.file_path).await
    }

    pub async fn write(&self, content: &str) -> Result<(), std::io::Error> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&self.file_path, content).await
    }

    pub async fn append_log(&self, entry: &str) -> Result<(), std::io::Error> {
        let mut current = self.read().await?;
        let timestamp = chrono::Utc::now().to_rfc3339();
        current.push_str(&format!("\n[{}] {}", timestamp, entry));
        self.write(&current).await
    }
}
