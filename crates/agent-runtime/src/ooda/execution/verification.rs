use super::git::capture_git_diff;
use super::SubtaskExecutor;
use crate::verifier::TaskVerifier;
use agent_domain::{AgentId, InterAgentEvent, SubTask, Task, TaskStatus, VerificationResult};
use std::path::Path;
use tracing::{info, warn};

/// Kết quả sau khi thực thi TaskVerifier cho một subtask
#[derive(Debug, Clone, PartialEq)]
pub enum SubtaskVerificationOutcome {
    Passed {
        verif_results: Vec<VerificationResult>,
    },
    Failed {
        error: String,
        verif_results: Vec<VerificationResult>,
    },
    NeedsReview { reason: String, verif_results: Vec<VerificationResult> },
}

/// Thực thi quy trình kiểm tra TaskVerifier đối với subtask vừa hoàn tất lệnh exit code 0
pub async fn execute_subtask_verification(
    subtask: &mut SubTask,
    active_workdir: &Path,
    all_verifications: &mut Vec<VerificationResult>,
) -> SubtaskVerificationOutcome {
    execute_with_verifier(subtask, active_workdir, all_verifications, TaskVerifier::new(active_workdir), false).await
}

async fn execute_with_verifier(
    subtask: &mut SubTask,
    _active_workdir: &Path,
    all_verifications: &mut Vec<VerificationResult>,
    verifier: TaskVerifier<'_>,
    auto_approve: bool,
) -> SubtaskVerificationOutcome {
    // Chuyển trạng thái sang SelfChecking
    subtask.status = TaskStatus::SelfChecking;
    info!("🔍 [OODA] SubTask {} in SelfChecking phase", subtask.id);

    let specs = if let Some(ref spec) = subtask.verification_spec {
        vec![spec.clone()]
    } else {
        verifier.parse_specs(&subtask.expected_verification)
    };

    let (verified_ok, verif_results) = verifier.verify_all(&specs).await;
    all_verifications.extend(verif_results.clone());

    let failed_automatic = verif_results.iter().any(|r| !r.passed && !matches!(r.spec, agent_domain::VerificationSpec::Custom { .. }));
    if !failed_automatic && (specs.is_empty() || verif_results.iter().any(|r| matches!(r.spec, agent_domain::VerificationSpec::Custom { .. }))) {
        if auto_approve {
            info!("✅ [OODA] Auto-approving custom/unspecified verification for subtask {}", subtask.id);
            SubtaskVerificationOutcome::Passed { verif_results }
        } else {
            SubtaskVerificationOutcome::NeedsReview { reason: "Manual verification required".into(), verif_results }
        }
    } else if verified_ok {
        SubtaskVerificationOutcome::Passed { verif_results }
    } else {
        let failure_details = verif_results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| r.details.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let err_msg = format!("Verification failed: {}", failure_details);
        warn!(
            "⚠️ [OODA] SubTask {} verification failed: {}",
            subtask.id, err_msg
        );
        SubtaskVerificationOutcome::Failed {
            error: err_msg,
            verif_results,
        }
    }
}

/// Cập nhật trạng thái hoàn thành thành công của subtask sau khi vượt qua bước kiểm tra TaskVerifier
#[allow(clippy::too_many_arguments)]
pub async fn finalize_subtask_completion(
    executor: &SubtaskExecutor,
    task: &mut Task,
    subtask_index: usize,
    mut subtask: SubTask,
    excerpt: String,
    verif_results: Vec<VerificationResult>,
    duration_ms: u128,
    active_workdir: &Path,
) -> Result<(), String> {
    subtask.status = TaskStatus::Completed;
    subtask.output_excerpt = Some(excerpt);
    if let Some(first_res) = verif_results.into_iter().next() {
        subtask.verification_result = Some(first_res);
    }
    info!(
        "✅ [OODA] SubTask {} verified and completed (exit 0 and verification passed).",
        subtask.id
    );

    if let Some(diff) = capture_git_diff(active_workdir).await {
        info!("📝 [OODA Git Diff Tracking]:\n{}", diff);
        if let Some(ref bus) = executor.bus {
            let _ = bus.publish(InterAgentEvent::new(
                AgentId::from_str("ooda-overseer"),
                None,
                "task.git_diff",
                serde_json::json!({
                    "task_id": task.id.0,
                    "subtask_id": subtask.id,
                    "diff": diff,
                }),
            ));
        }
        let _ = executor
            .scratchpad
            .append_log(&format!(
                "Git changes detected after {}:\n{}",
                subtask.id, diff
            ))
            .await;
    }

    executor
        .scratchpad
        .append_log(&format!(
            "SubTask {} completed successfully in {}ms",
            subtask.id, duration_ms
        ))
        .await
        .map_err(|e| e.to_string())?;

    if let Some(slot) = task.subtasks.get_mut(subtask_index) {
        *slot = subtask;
    } else {
        return Err(format!(
            "Subtask index {} out of bounds (subtasks len: {})",
            subtask_index,
            task.subtasks.len()
        ));
    }
    Ok(())
}

impl SubtaskExecutor {
    pub async fn execute_subtask_verification(
        &self,
        subtask: &mut SubTask,
        active_workdir: &Path,
        all_verifications: &mut Vec<VerificationResult>,
    ) -> SubtaskVerificationOutcome {
        let verifier = TaskVerifier::with_policy(active_workdir, self.sandbox_config.clone().unwrap_or_default(), self.guardrail.clone());
        execute_with_verifier(subtask, active_workdir, all_verifications, verifier, self.auto_approve).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finalize_subtask_completion(
        &self,
        task: &mut Task,
        subtask_index: usize,
        subtask: SubTask,
        excerpt: String,
        verif_results: Vec<VerificationResult>,
        duration_ms: u128,
        active_workdir: &Path,
    ) -> Result<(), String> {
        finalize_subtask_completion(
            self,
            task,
            subtask_index,
            subtask,
            excerpt,
            verif_results,
            duration_ms,
            active_workdir,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_domain::VerificationSpec;

    #[tokio::test]
    async fn test_execute_subtask_verification_passed() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = std::env::temp_dir().join(format!("test_verif_pass_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join("hello.txt");
        std::fs::write(&test_file, "content")?;

        let mut st = SubTask::new("1", "Check hello", "echo ok", "file: hello.txt");
        st.verification_spec = Some(VerificationSpec::FileExists {
            path: "hello.txt".into(),
            min_bytes: None,
        });

        let mut all_verifs = Vec::new();
        let outcome = execute_subtask_verification(&mut st, &temp_dir, &mut all_verifs).await;

        assert_eq!(st.status, TaskStatus::SelfChecking);
        assert!(matches!(outcome, SubtaskVerificationOutcome::Passed { .. }));
        assert_eq!(all_verifs.len(), 1);
        assert!(all_verifs[0].passed);

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_execute_subtask_verification_failed() {
        let temp_dir = std::env::temp_dir().join(format!("test_verif_fail_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut st = SubTask::new("1", "Check missing", "echo ok", "file: missing.txt");
        st.verification_spec = Some(VerificationSpec::FileExists {
            path: "missing.txt".into(),
            min_bytes: None,
        });

        let mut all_verifs = Vec::new();
        let outcome = execute_subtask_verification(&mut st, &temp_dir, &mut all_verifs).await;

        assert_eq!(st.status, TaskStatus::SelfChecking);
        assert!(matches!(outcome, SubtaskVerificationOutcome::Failed { .. }));
        if let SubtaskVerificationOutcome::Failed { error, verif_results } = outcome {
            assert!(error.contains("Verification failed"));
            assert_eq!(verif_results.len(), 1);
            assert!(!verif_results[0].passed);
        }
        assert_eq!(all_verifs.len(), 1);
        assert!(!all_verifs[0].passed);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
