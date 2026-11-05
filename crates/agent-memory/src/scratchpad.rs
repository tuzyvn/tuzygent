use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone)]
pub struct ScratchpadManager {
    file_path: PathBuf,
}

impl ScratchpadManager {
    pub fn new<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self {
            file_path: workspace_dir.as_ref().join("memory").join("scratchpad.md"),
        }
    }

    pub fn for_task<P: AsRef<Path>>(workspace_dir: P, task_id: &str) -> Self {
        let safe_id = task_id.replace(['/', '\\', ' '], "_");
        Self {
            file_path: workspace_dir
                .as_ref()
                .join("memory")
                .join(format!("scratchpad_{}.md", safe_id)),
        }
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
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

    /// Ghi log tiến trình thực thi trực tiếp bằng chế độ atomic append, tránh race condition
    pub async fn append_log(&self, entry: &str) -> Result<(), std::io::Error> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let timestamp = chrono::Utc::now().to_rfc3339();
        let log_line = format!("[{}] {}\n", timestamp, entry.trim());

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
            .await?;

        file.write_all(log_line.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// Đọc n nhật ký gần nhất phục vụ Working Memory của Agent trong vòng lặp OODA
    pub async fn read_recent_entries(&self, n: usize) -> Result<Vec<String>, std::io::Error> {
        let content = self.read().await?;
        let lines: Vec<String> = content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        let start = lines.len().saturating_sub(n);
        Ok(lines[start..].to_vec())
    }

    /// Làm rỗng scratchpad khi chuyển đổi phiên làm việc
    pub async fn clear(&self) -> Result<(), std::io::Error> {
        if self.file_path.exists() {
            fs::remove_file(&self.file_path).await?;
        }
        Ok(())
    }
}
