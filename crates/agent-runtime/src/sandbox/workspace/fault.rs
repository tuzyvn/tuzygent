use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;

/// Các loại lỗi hỗn loạn mô phỏng trong vòng đời workspace sandbox
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceFaultKind {
    None,
    GitIndexLock,
    TransientIo,
    SocketTimeout,
    AbruptTermination,
    LinkerOomContention,
    CorruptedPath,
}

/// Các lỗi phát sinh trong quá trình chuẩn bị hoặc dọn dẹp workspace
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    #[error("Git lock file contention (.git/index.lock held)")]
    GitLockContention,
    #[error("Transient workspace I/O failure: {0}")]
    TransientIo(String),
    #[error("Workspace socket timeout")]
    SocketTimeout,
    #[error("Process aborted with exit code {0}")]
    AbruptExit(i32),
    #[error("Workspace linker memory contention / collect2 fatal error")]
    LinkerOom,
    #[error("Corrupted workspace path: {0}")]
    CorruptedPath(String),
    #[error("Capacity limit exceeded: {0}")]
    CapacityExceeded(String),
    #[error("Permanent failure: {0}")]
    Permanent(String),
}

/// Phân loại lỗi có thể phục hồi tự động bằng cơ chế retry hay không
pub fn is_retryable_workspace_error(err: &WorkspaceError) -> bool {
    matches!(
        err,
        WorkspaceError::GitLockContention
            | WorkspaceError::TransientIo(_)
            | WorkspaceError::SocketTimeout
            | WorkspaceError::LinkerOom
    )
}

/// Chính sách tiêm lỗi hỗn loạn (Fault Injection Policy)
#[derive(Debug, Clone)]
pub struct WorkspaceFaultPolicy {
    kind: WorkspaceFaultKind,
    max_injections: usize,
    injected_count: Arc<AtomicUsize>,
}

impl WorkspaceFaultPolicy {
    pub fn none() -> Self {
        Self {
            kind: WorkspaceFaultKind::None,
            max_injections: 0,
            injected_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn intermittent(kind: WorkspaceFaultKind, max_injections: usize) -> Self {
        Self {
            kind,
            max_injections,
            injected_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn check_and_inject(&self) -> Result<(), WorkspaceError> {
        if self.kind == WorkspaceFaultKind::None {
            return Ok(());
        }
        let count = self.injected_count.fetch_add(1, Ordering::SeqCst);
        if count < self.max_injections {
            match self.kind {
                WorkspaceFaultKind::GitIndexLock => Err(WorkspaceError::GitLockContention),
                WorkspaceFaultKind::TransientIo => {
                    Err(WorkspaceError::TransientIo("disk sync I/O error".into()))
                }
                WorkspaceFaultKind::SocketTimeout => Err(WorkspaceError::SocketTimeout),
                WorkspaceFaultKind::AbruptTermination => Err(WorkspaceError::AbruptExit(2)),
                WorkspaceFaultKind::LinkerOomContention => Err(WorkspaceError::LinkerOom),
                WorkspaceFaultKind::CorruptedPath => {
                    Err(WorkspaceError::CorruptedPath("nonexistent/ws".into()))
                }
                WorkspaceFaultKind::None => Ok(()),
            }
        } else {
            Ok(())
        }
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.injected_count.store(0, Ordering::SeqCst);
    }
}
