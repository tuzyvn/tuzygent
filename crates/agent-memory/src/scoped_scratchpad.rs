use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Quyền truy cập ngữ cảnh cho Scope của Sub-Agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeAccess {
    /// Chỉ đọc: Sub-agent chỉ được đọc dữ liệu cha, không thể ghi/append vào scratchpad
    ReadOnly,
    /// Đọc/Ghi cục bộ: Sub-agent có vùng nhớ scratchpad riêng biệt, không ghi đè parent
    ReadWrite,
    /// Cô lập hoàn toàn: Sub-agent có vùng nhớ riêng, bị cấm đọc trộm bộ nhớ cha (PermissionDenied)
    Isolated,
}

/// Bộ nhớ tạm phân cấp có phân quyền và cô lập (Scoped Scratchpad Isolation)
#[derive(Debug, Clone)]
pub struct ScopedScratchpad {
    scope_id: String,
    parent_scope_id: Option<String>,
    file_path: PathBuf,
    access: ScopeAccess,
    parent_path: Option<PathBuf>,
    auto_cleanup: bool,
}

impl ScopedScratchpad {
    /// Khởi tạo ScopedScratchpad độc lập từ thư mục gốc
    pub fn new<P: AsRef<Path>>(
        workspace_dir: P,
        scope_id: impl Into<String>,
        access: ScopeAccess,
    ) -> Self {
        let scope_id = scope_id.into();
        let safe_id = scope_id.replace(['/', '\\', ' '], "_");
        let file_path = workspace_dir
            .as_ref()
            .join("memory")
            .join("scopes")
            .join(format!("scratchpad_scope_{}.md", safe_id));
        Self {
            scope_id,
            parent_scope_id: None,
            file_path,
            access,
            parent_path: None,
            auto_cleanup: false,
        }
    }

    /// Khởi tạo ScopedScratchpad phân cấp kế thừa từ scratchpad của parent
    pub fn from_parent<P: AsRef<Path>>(
        parent_path: P,
        parent_scope_id: impl Into<String>,
        child_id: impl Into<String>,
        access: ScopeAccess,
    ) -> Self {
        let parent_path = parent_path.as_ref().to_path_buf();
        let parent_scope_id = parent_scope_id.into();
        let child_id = child_id.into();
        let safe_parent = parent_scope_id.replace(['/', '\\', ' '], "_");
        let safe_child = child_id.replace(['/', '\\', ' '], "_");

        let parent_dir = parent_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let file_path = parent_dir
            .join("scopes")
            .join(safe_parent.clone())
            .join(format!("{}.md", safe_child));

        Self {
            scope_id: child_id,
            parent_scope_id: Some(parent_scope_id),
            file_path,
            access,
            parent_path: Some(parent_path),
            auto_cleanup: false,
        }
    }

    /// Sinh scope con cấp tiếp theo (Hierarchical Sub-delegation)
    pub fn child_scope(&self, child_id: &str, access: ScopeAccess) -> Self {
        Self::from_parent(&self.file_path, &self.scope_id, child_id, access)
    }

    /// Sinh scope con bị cô lập hoàn toàn, cấm đọc trộm bộ nhớ cha
    pub fn isolated_child(&self, child_id: &str) -> Self {
        self.child_scope(child_id, ScopeAccess::Isolated)
    }

    /// Bật/tắt chế độ tự động xóa tệp scratchpad tạm thời khi struct bị Drop
    pub fn with_auto_cleanup(mut self, auto_cleanup: bool) -> Self {
        self.auto_cleanup = auto_cleanup;
        self
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    pub fn parent_scope_id(&self) -> Option<&str> {
        self.parent_scope_id.as_deref()
    }

    pub fn access(&self) -> ScopeAccess {
        self.access
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn parent_path(&self) -> Option<&Path> {
        self.parent_path.as_deref()
    }

    /// Đọc toàn bộ nội dung trong scope scratchpad cục bộ
    pub async fn read(&self) -> Result<String, std::io::Error> {
        if !self.file_path.exists() {
            return Ok(String::new());
        }
        fs::read_to_string(&self.file_path).await
    }

    /// Đọc nội dung scratchpad của parent task (nếu có quyền và tệp tồn tại)
    pub async fn read_parent(&self) -> Result<String, std::io::Error> {
        if self.access == ScopeAccess::Isolated {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "Scoped scratchpad '{}' is Isolated: reading parent memory is forbidden",
                    self.scope_id
                ),
            ));
        }
        match &self.parent_path {
            Some(path) => {
                if !path.exists() {
                    return Ok(String::new());
                }
                fs::read_to_string(path).await
            }
            None => Ok(String::new()),
        }
    }

    /// Ghi nội dung vào scratchpad cục bộ (chặn nếu quyền ReadOnly)
    pub async fn write(&self, content: &str) -> Result<(), std::io::Error> {
        if self.access == ScopeAccess::ReadOnly {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Scoped scratchpad '{}' is ReadOnly", self.scope_id),
            ));
        }
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&self.file_path, content).await
    }

    /// Ghi log tiến trình có tiền tố scope_id vào scratchpad cục bộ (chặn nếu quyền ReadOnly)
    pub async fn append_log(&self, entry: &str) -> Result<(), std::io::Error> {
        if self.access == ScopeAccess::ReadOnly {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Scoped scratchpad '{}' is ReadOnly", self.scope_id),
            ));
        }
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let timestamp = chrono::Utc::now().to_rfc3339();
        let log_line = format!("[{}] [{}] {}\n", timestamp, self.scope_id, entry.trim());

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
            .await?;

        file.write_all(log_line.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// Đọc n nhật ký gần nhất phục vụ context ngắn hạn
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

    /// Hợp nhất kết quả ủy quyền (Delegation Result) vào tệp scratchpad của parent
    pub async fn merge_into_parent(&self, summary: &str) -> Result<(), std::io::Error> {
        let parent_path = match &self.parent_path {
            Some(path) => path,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Scoped scratchpad '{}' has no parent to merge into", self.scope_id),
                ));
            }
        };

        if let Some(parent_dir) = parent_path.parent() {
            fs::create_dir_all(parent_dir).await?;
        }

        let timestamp = chrono::Utc::now().to_rfc3339();
        let log_line = format!(
            "[{}] [DELEGATION RESULT: {}] {}\n",
            timestamp,
            self.scope_id,
            summary.trim()
        );

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(parent_path)
            .await?;

        file.write_all(log_line.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// Xóa tệp scratchpad cục bộ thủ công khi hoàn thành nhiệm vụ
    pub async fn cleanup(&self) -> Result<(), std::io::Error> {
        if self.file_path.exists() {
            fs::remove_file(&self.file_path).await?;
        }
        Ok(())
    }
}

impl Drop for ScopedScratchpad {
    fn drop(&mut self) {
        if self.auto_cleanup && self.file_path.exists() {
            let _ = std::fs::remove_file(&self.file_path);
        }
    }
}
