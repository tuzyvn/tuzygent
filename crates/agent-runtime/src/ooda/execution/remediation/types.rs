use crate::ooda::execution::runner::QueueExecutionOutcome;

/// Phân loại nguyên nhân lỗi kích hoạt quy trình tự sửa sai (Remediation)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemediationFailureKind {
    Verification,
    ExitCode,
    EngineError,
}

/// Kết quả của việc thử nghiệm xử lý khắc phục lỗi subtask
#[derive(Debug, Clone, PartialEq)]
pub enum RemediationOutcome {
    /// Đã tạo thành công Remediation SubTask và chèn vào đầu vị trí hiện tại (Backtracking Re-Act).
    /// Bộ điều phối cần tiếp tục vòng lặp (`continue`).
    BacktrackingInserted,
    /// Hết lượt thử lại hoặc không sinh được bước sửa sai, nhiệm vụ chuyển sang thất bại.
    Failed(QueueExecutionOutcome),
}
