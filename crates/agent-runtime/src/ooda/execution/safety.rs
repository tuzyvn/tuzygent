use super::SubtaskExecutor;
use agent_domain::{GuardrailPolicy, Task, TaskStatus};
use agent_memory::DurableMemoryStore;
use tracing::warn;

/// Represents the reason why a subtask command is blocked by guardrail or safety policies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailBlockReason {
    /// The command triggered a hard red-line guardrail violation (e.g. destructive command).
    RedLine(String),
    /// The command triggered a shell policy violation (e.g. shell execution is prohibited).
    ShellProhibited(String),
}

impl GuardrailBlockReason {
    /// Returns the formatted approval reason string suitable for `QueueExecutionOutcome::BlockedWaitingApproval`.
    pub fn approval_reason(&self) -> String {
        match self {
            Self::RedLine(err) => format!("Task blocked waiting approval: {}", err),
            Self::ShellProhibited(err) => format!("Task blocked: {}", err),
        }
    }

    /// Returns the raw violation error message.
    pub fn error_message(&self) -> &str {
        match self {
            Self::RedLine(err) | Self::ShellProhibited(err) => err,
        }
    }
}

/// Evaluates whether a subtask command or prompt passes the guardrail policy checks.
/// Checks both red-line rules and shell execution restrictions.
pub fn evaluate_command_safety(
    guardrail: &GuardrailPolicy,
    command_or_prompt: &str,
) -> Result<(), GuardrailBlockReason> {
    // 1. Hard Red-line safety check
    if let Err(red_line_err) = guardrail.check_command_safety(command_or_prompt) {
        return Err(GuardrailBlockReason::RedLine(red_line_err));
    }

    // 2. Shell execution policy check
    if !guardrail.allow_shell_execution {
        let cmd_lower = command_or_prompt.to_lowercase();
        if cmd_lower.starts_with("sh ")
            || cmd_lower.starts_with("bash ")
            || cmd_lower.starts_with("zsh ")
        {
            return Err(GuardrailBlockReason::ShellProhibited(
                "Shell execution is prohibited by guardrail policy".to_string(),
            ));
        }
    }

    Ok(())
}

/// Alias for `evaluate_command_safety` for backward and semantic compatibility.
pub fn evaluate_guardrail_safety(
    guardrail: &GuardrailPolicy,
    command_or_prompt: &str,
) -> Result<(), GuardrailBlockReason> {
    evaluate_command_safety(guardrail, command_or_prompt)
}

/// Checks the subtask at `subtask_index` against the guardrail policy.
///
/// If blocked:
/// - Emits a warning log.
/// - Sets both the subtask and task statuses to `TaskStatus::WaitingApproval`.
/// - Records a lesson to durable memory (for red-line violations).
/// - Returns `Err(approval_reason)`.
///
/// If safe:
/// - Returns `Ok(())`.
pub async fn check_and_apply_guardrail_policy(
    guardrail: &GuardrailPolicy,
    durable: &DurableMemoryStore,
    task: &mut Task,
    subtask_index: usize,
) -> Result<(), String> {
    let subtask = match task.subtasks.get_mut(subtask_index) {
        Some(s) => s,
        None => {
            return Err(format!(
                "Subtask index {} out of bounds (subtasks len: {})",
                subtask_index,
                task.subtasks.len()
            ))
        }
    };

    match evaluate_command_safety(guardrail, &subtask.command_or_prompt) {
        Ok(()) => Ok(()),
        Err(reason) => {
            warn!("🚨 [GUARDRAIL BLOCK]: {}", reason.error_message());
            let err_msg = reason.error_message().to_string();
            subtask.status = TaskStatus::WaitingApproval {
                reason: err_msg.clone(),
            };
            task.status = TaskStatus::WaitingApproval { reason: err_msg };

            if let GuardrailBlockReason::RedLine(ref red_line_err) = reason {
                let _ = durable
                    .save_lesson(&format!(
                        "Guardrail blocked command in task '{}': {}",
                        task.title, red_line_err
                    ))
                    .await;
            }

            Err(reason.approval_reason())
        }
    }
}

impl SubtaskExecutor {
    pub fn evaluate_guardrail_safety(&self, command: &str) -> Result<(), GuardrailBlockReason> {
        evaluate_guardrail_safety(&self.guardrail, command)
    }

    pub async fn check_and_apply_guardrail_policy(
        &self,
        task: &mut Task,
        subtask_index: usize,
    ) -> Result<(), String> {
        check_and_apply_guardrail_policy(&self.guardrail, &self.durable, task, subtask_index).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_domain::{Priority, SubTask};

    #[test]
    fn test_evaluate_command_safety_safe_command() {
        let guardrail = GuardrailPolicy::default_enterprise();
        assert!(evaluate_command_safety(&guardrail, "cargo test --all").is_ok());
    }

    #[test]
    fn test_evaluate_command_safety_red_line_violation() {
        let guardrail = GuardrailPolicy::default_enterprise();
        let result = evaluate_command_safety(&guardrail, "rm -rf /");
        assert!(matches!(result, Err(GuardrailBlockReason::RedLine(_))));
        if let Err(reason) = result {
            assert!(reason
                .approval_reason()
                .contains("Task blocked waiting approval"));
        }
    }

    #[test]
    fn test_evaluate_command_safety_shell_policy() {
        let mut guardrail = GuardrailPolicy::default_enterprise();
        guardrail.allow_shell_execution = false;

        let result = evaluate_command_safety(&guardrail, "bash run_script.sh");
        assert!(matches!(
            result,
            Err(GuardrailBlockReason::ShellProhibited(_))
        ));
        if let Err(reason) = result {
            assert!(reason
                .approval_reason()
                .contains("Task blocked: Shell execution is prohibited"));
        }

        guardrail.allow_shell_execution = true;
        assert!(evaluate_command_safety(&guardrail, "bash run_script.sh").is_ok());
    }

    #[tokio::test]
    async fn test_check_and_apply_guardrail_policy_blocks_task(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let unique_dir =
            std::env::temp_dir().join(format!("test_safety_guardrail_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&unique_dir);
        let db_path = unique_dir.join("durable_test.db");
        let durable = DurableMemoryStore::new(&db_path);

        let guardrail = GuardrailPolicy::default_enterprise();
        let mut task = Task::new("Deploy App", "Deploy dangerous code", Priority::High);
        task.subtasks.push(SubTask::new(
            "1",
            "Cleanup",
            "rm -rf /tmp/dangerous",
            "exit 0",
        ));

        let result = check_and_apply_guardrail_policy(&guardrail, &durable, &mut task, 0).await;
        assert!(result.is_err());
        assert!(matches!(task.status, TaskStatus::WaitingApproval { .. }));
        assert!(matches!(
            task.subtasks[0].status,
            TaskStatus::WaitingApproval { .. }
        ));

        let memory = durable.load_long_term_context().await?;
        assert!(memory.contains("Guardrail blocked command"));

        let _ = std::fs::remove_dir_all(&unique_dir);
        Ok(())
    }
}
