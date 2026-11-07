use super::fault::{is_retryable_workspace_error, WorkspaceError, WorkspaceFaultPolicy};
use super::manager::WorkspaceManager;
use crate::sandbox::config::IsolationMode;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

/// Bản ghi nhật ký kiểm toán vòng đời workspace
#[derive(Debug, Clone)]
pub struct WorkspaceAuditRecord {
    pub task_id: String,
    pub path: PathBuf,
    pub mode: IsolationMode,
    pub retries_taken: usize,
    pub successful: bool,
    pub duration_us: u64,
}

/// Trình quản lý workspace tự phục hồi (Self-Healing Resilient Workspace Manager)
#[derive(Debug, Clone)]
pub struct ResilientWorkspaceManager {
    pub inner: WorkspaceManager,
    pub policy: WorkspaceFaultPolicy,
    pub max_retries: usize,
    pub backoff: Duration,
    pub max_capacity: usize,
    pub active_workspaces: Arc<Mutex<VecDeque<PathBuf>>>,
    pub audit_ledger: Arc<Mutex<Vec<WorkspaceAuditRecord>>>,
    pub recovery_count: Arc<Mutex<usize>>,
}

impl ResilientWorkspaceManager {
    pub fn new(inner: WorkspaceManager, policy: WorkspaceFaultPolicy, max_retries: usize) -> Self {
        Self {
            inner,
            policy,
            max_retries,
            backoff: Duration::from_millis(5),
            max_capacity: 5_000,
            active_workspaces: Arc::new(Mutex::new(VecDeque::new())),
            audit_ledger: Arc::new(Mutex::new(Vec::new())),
            recovery_count: Arc::new(Mutex::new(0)),
        }
    }

    pub fn with_default(base_repo_path: &Path, mode: IsolationMode) -> Self {
        Self::new(
            WorkspaceManager::new(base_repo_path, mode),
            WorkspaceFaultPolicy::none(),
            3,
        )
    }

    pub fn with_max_capacity(mut self, capacity: usize) -> Self {
        self.max_capacity = capacity;
        self
    }

    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    /// Khởi tạo workspace với cơ chế tự phục hồi, backoff retry và giới hạn dung lượng
    pub async fn prepare_resilient(&mut self, task_id: &str) -> Result<PathBuf, WorkspaceError> {
        let start = Instant::now();
        let mut retries = 0;

        loop {
            // Kiểm tra tiêm lỗi hỗn loạn
            if let Err(fault) = self.policy.check_and_inject() {
                if is_retryable_workspace_error(&fault) && retries < self.max_retries {
                    retries += 1;
                    if self.backoff > Duration::ZERO {
                        tokio::time::sleep(self.backoff).await;
                    }
                    continue;
                }
                self.record_audit(task_id, PathBuf::new(), retries, false, start.elapsed());
                return Err(fault);
            }

            // Thực thi khởi tạo
            match self.inner.prepare(task_id).await {
                Ok(path) => {
                    self.enforce_capacity_limit().await;
                    if let Ok(mut actives) = self.active_workspaces.lock() {
                        actives.push_back(path.clone());
                    }

                    if retries > 0 {
                        if let Ok(mut rec) = self.recovery_count.lock() {
                            *rec += 1;
                        }
                    }

                    self.record_audit(task_id, path.clone(), retries, true, start.elapsed());
                    return Ok(path);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let ws_err = if err_str.contains("index.lock") {
                        WorkspaceError::GitLockContention
                    } else if err_str.contains("timeout") {
                        WorkspaceError::SocketTimeout
                    } else if err_str.contains("exited with error code: 2") {
                        WorkspaceError::AbruptExit(2)
                    } else {
                        WorkspaceError::TransientIo(err_str)
                    };

                    if is_retryable_workspace_error(&ws_err) && retries < self.max_retries {
                        retries += 1;
                        warn!(
                            "Transient workspace failure, retrying ({}/{}): {}",
                            retries, self.max_retries, ws_err
                        );
                        if self.backoff > Duration::ZERO {
                            tokio::time::sleep(self.backoff).await;
                        }
                        continue;
                    }

                    self.record_audit(task_id, PathBuf::new(), retries, false, start.elapsed());
                    return Err(ws_err);
                }
            }
        }
    }

    /// Dọn dẹp workspace hiện tại với cơ chế kiểm soát ngoại lệ
    pub async fn teardown_resilient(&mut self) -> Result<(), WorkspaceError> {
        if let Err(fault) = self.policy.check_and_inject() {
            if is_retryable_workspace_error(&fault) {
                // Thử lại lần thứ hai cho thao tác teardown
                let _ = self.inner.teardown().await;
                return Ok(());
            }
            return Err(fault);
        }

        let _ = self.inner.teardown().await;
        if let Ok(mut actives) = self.active_workspaces.lock() {
            if let Some(target) = &self.inner.worktree_path {
                actives.retain(|p| p != target);
            } else {
                actives.pop_front();
            }
        }
        Ok(())
    }

    /// Chuẩn bị hàng loạt workspace theo batch
    pub async fn batch_prepare(
        &mut self,
        task_ids: &[&str],
    ) -> Vec<Result<PathBuf, WorkspaceError>> {
        let mut results = Vec::with_capacity(task_ids.len());
        for task_id in task_ids {
            results.push(self.prepare_resilient(task_id).await);
        }
        results
    }

    /// Giới hạn số lượng workspace hoạt động đồng thời (FIFO Eviction)
    async fn enforce_capacity_limit(&mut self) {
        let mut excess_paths = Vec::new();
        if let Ok(mut actives) = self.active_workspaces.lock() {
            while actives.len() >= self.max_capacity {
                if let Some(p) = actives.pop_front() {
                    excess_paths.push(p);
                }
            }
        }
        for path in excess_paths {
            if path.exists() && path.starts_with(std::env::temp_dir()) {
                let _ = tokio::fs::remove_dir_all(&path).await;
            }
        }
    }

    fn record_audit(
        &self,
        task_id: &str,
        path: PathBuf,
        retries: usize,
        success: bool,
        duration: Duration,
    ) {
        if let Ok(mut ledger) = self.audit_ledger.lock() {
            while ledger.len() >= self.max_capacity {
                ledger.remove(0);
            }
            ledger.push(WorkspaceAuditRecord {
                task_id: task_id.to_string(),
                path,
                mode: self.inner.mode,
                retries_taken: retries,
                successful: success,
                duration_us: duration.as_micros() as u64,
            });
        }
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let active_count = self.active_workspaces.lock().map(|a| a.len()).unwrap_or(0);
        let audit_count = self.audit_ledger.lock().map(|l| l.len()).unwrap_or(0);
        active_count * 128 + audit_count * 256
    }

    pub fn recovery_count(&self) -> usize {
        self.recovery_count.lock().map(|r| *r).unwrap_or(0)
    }

    pub fn active_count(&self) -> usize {
        self.active_workspaces.lock().map(|a| a.len()).unwrap_or(0)
    }

    pub fn audit_records(&self) -> Vec<WorkspaceAuditRecord> {
        self.audit_ledger
            .lock()
            .map(|l| l.clone())
            .unwrap_or_default()
    }
}
