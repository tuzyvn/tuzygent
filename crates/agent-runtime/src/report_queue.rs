use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TerminalReportKind {
    Completed,
    Failed,
    Aborted,
}

/// Bản ghi kết quả cuối cùng của tác vụ cần báo cáo về cho Gateway/Control Server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalTaskReport {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub kind: TerminalReportKind,
    pub task_id: String,
    pub agent_id: String,
    pub output: Option<String>,
    pub error_message: Option<String>,
    pub retry_count: u32,
    pub failure_reason: Option<String>,
}

/// Outbox bền vững lưu trữ trên đĩa để đảm bảo kết quả báo cáo không bao giờ bị mất
/// ngay cả khi mạng sập hoặc daemon bị ngắt đột ngột (Multica Pattern)
pub struct TerminalReportStore {
    queue_dir: PathBuf,
    failed_dir: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl TerminalReportStore {
    pub fn new<P: AsRef<Path>>(workspaces_root: P) -> Self {
        let base = workspaces_root
            .as_ref()
            .join(".pending-terminal-reports")
            .join("v1");
        let failed = base.join("quarantined");
        Self {
            queue_dir: base,
            failed_dir: failed,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn init(&self) -> Result<(), std::io::Error> {
        fs::create_dir_all(&self.queue_dir).await?;
        fs::create_dir_all(&self.failed_dir).await?;
        Ok(())
    }

    /// Tạo tên file an toàn bằng băm SHA256 để chống Path Traversal từ Task ID
    fn report_file_name(task_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(task_id.as_bytes());
        let hash = hasher.finalize();
        format!("{:x}.json", hash)
    }

    /// Ghi báo cáo vào outbox bền vững trước khi gửi qua mạng (Write-Ahead Queue)
    pub async fn enqueue(&self, report: &TerminalTaskReport) -> Result<PathBuf, std::io::Error> {
        let _guard = self.lock.lock().await;
        self.init().await?;

        let file_name = Self::report_file_name(&report.task_id);
        let target_path = self.queue_dir.join(&file_name);
        let temp_path = self.queue_dir.join(format!("{}.tmp", file_name));

        let data = serde_json::to_vec_pretty(report)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // Ghi nguyên tử qua file tạm rồi rename (Atomic Write)
        fs::write(&temp_path, &data).await?;
        fs::rename(&temp_path, &target_path).await?;

        info!(
            "💾 [ReportStore] Enqueued terminal report for task '{}' at {:?}",
            report.task_id, target_path
        );
        Ok(target_path)
    }

    /// Xoá báo cáo sau khi máy chủ đã xác nhận nhận thành công (ACK)
    pub async fn acknowledge(&self, task_id: &str) -> Result<(), std::io::Error> {
        let _guard = self.lock.lock().await;
        let file_name = Self::report_file_name(task_id);
        let path = self.queue_dir.join(file_name);
        if path.exists() {
            fs::remove_file(&path).await?;
            info!(
                "✅ [ReportStore] Acknowledged and removed terminal report for task '{}'",
                task_id
            );
        }
        Ok(())
    }

    /// Quét các báo cáo đang tồn đọng trong hàng đợi chưa gửi thành công
    pub async fn list_pending(&self) -> Result<Vec<TerminalTaskReport>, std::io::Error> {
        let _guard = self.lock.lock().await;
        let mut pending = Vec::new();
        if !self.queue_dir.exists() {
            return Ok(pending);
        }

        let mut entries = fs::read_dir(&self.queue_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path).await {
                    if let Ok(report) = serde_json::from_str::<TerminalTaskReport>(&content) {
                        pending.push(report);
                    }
                }
            }
        }
        Ok(pending)
    }

    /// Cách ly báo cáo bị từ chối vĩnh viễn (Quarantine)
    pub async fn quarantine(&self, task_id: &str) -> Result<(), std::io::Error> {
        let _guard = self.lock.lock().await;
        let file_name = Self::report_file_name(task_id);
        let source_path = self.queue_dir.join(&file_name);
        let target_path = self.failed_dir.join(&file_name);
        if source_path.exists() {
            fs::rename(&source_path, &target_path).await?;
            warn!(
                "☣️ [ReportStore] Quarantined rejected report for task '{}' to {:?}",
                task_id, target_path
            );
        }
        Ok(())
    }
}
