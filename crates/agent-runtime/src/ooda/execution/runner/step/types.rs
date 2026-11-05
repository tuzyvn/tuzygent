use crate::ooda::execution::remediation::RemediationOutcome;

/// Outcome of running a subtask queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueExecutionOutcome {
    Completed,
    BlockedWaitingApproval { reason: String },
    Canceled { reason: String },
    Failed { error: String },
}

/// Returns true if the message indicates a fatal authentication or quota error.
pub fn is_auth_or_quota_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("quota exhausted")
        || lower.contains("quota exceeded")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("401 unauthorized")
        || lower.contains("403 forbidden")
        || lower.contains("invalid api key")
        || lower.contains("unauthorized")
        || lower.contains("insufficient_quota")
        || lower.contains("resource_exhausted")
        || lower.contains("402 payment")
        || lower.contains("model not found")
        || lower.contains("model_not_found")
        || lower.contains("billing_not_active")
        || lower.contains("credit balance")
}

/// Action to be taken by the runner loop after handling a subtask execution step.
#[derive(Debug, PartialEq, Eq)]
pub enum StepAction {
    /// Step finished successfully (verified and finalized); advance to next subtask.
    Advance,
    /// Backtracking subtask was inserted; repeat current loop iteration without advancing index.
    Backtrack,
    /// Step encountered a fatal error or unrecoverable failure; terminate queue execution.
    Halt(QueueExecutionOutcome),
}

impl From<RemediationOutcome> for StepAction {
    fn from(outcome: RemediationOutcome) -> Self {
        match outcome {
            RemediationOutcome::BacktrackingInserted => StepAction::Backtrack,
            RemediationOutcome::Failed(queue_outcome) => StepAction::Halt(queue_outcome),
        }
    }
}
