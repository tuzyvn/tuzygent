use super::TaskService;
use agent_domain::{Task, TaskId, TaskResult, TaskStatus};
use agent_engine::ComputeEngine;
use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::sync::Arc;

impl TaskService {
    /// Approve a task that is currently in WaitingApproval status, resuming execution
    pub async fn approve_task(&self, id: &TaskId, _engine: Arc<dyn ComputeEngine>) -> Result<bool> {
        if let Some(task) = self.store.get_task(id).await? {
            if matches!(task.status, TaskStatus::WaitingApproval { .. }) {
                if matches!(&task.status, TaskStatus::WaitingApproval { reason } if reason.starts_with("Interrupted execution") || reason.starts_with("Recovery requires"))
                {
                    return Err(anyhow!(
                        "Recovery requires reconciliation; automatic retry is disabled"
                    ));
                }
                if task.subtasks.iter().any(|st| matches!(&st.status, TaskStatus::WaitingApproval { reason } if reason == "Manual verification required")) {
                    return Err(anyhow!("Manual verification review is not yet supported; the task will not be re-executed"));
                }
                return Err(anyhow!(
                    "Approval of policy-blocked actions is not supported"
                ));
            } else {
                return Err(anyhow!(
                    "Task is not in WaitingApproval status (current: {:?})",
                    task.status
                ));
            }
        }
        Ok(false)
    }

    /// Reject a task that is currently in WaitingApproval status
    pub async fn reject_task(&self, id: &TaskId, reason: &str) -> Result<bool> {
        if let Some(mut task) = self.store.get_task(id).await? {
            if matches!(task.status, TaskStatus::WaitingApproval { .. }) {
                let reason_str = if reason.trim().is_empty() {
                    "Rejected by reviewer".to_string()
                } else {
                    reason.trim().to_string()
                };
                task.status = TaskStatus::Failed {
                    error: format!("Review rejected: {}", reason_str),
                };
                task.updated_at = chrono::Utc::now();
                let result = TaskResult {
                    task_id: id.clone(),
                    status: task.status.clone(),
                    summary: format!("Review rejected: {}", reason_str),
                    artifacts: Vec::new(),
                    verifications: Vec::new(),
                    duration_ms: 0,
                    engine_used: task.engine_name.clone(),
                };
                self.store.finish_task(&task, &result, false).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn review_revision(&self, task: &Task) -> Result<String> {
        let bytes =
            serde_json::to_vec(&(task, &self.guardrail, format!("{:?}", self.sandbox_config)))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    /// Review only a completed engine step awaiting human verification; never re-execute it.
    pub async fn review_task(
        &self,
        id: &TaskId,
        revision: &str,
        reviewer: &str,
        approved: bool,
    ) -> Result<bool> {
        if reviewer.trim().is_empty() || reviewer.len() > 200 {
            return Err(anyhow!("A reviewer label of at most 200 bytes is required"));
        }
        if self
            .sandbox_config
            .as_ref()
            .is_some_and(|cfg| cfg.mode != crate::sandbox::IsolationMode::LocalDirectory)
        {
            return Err(anyhow!("Review resume requires the original workspace; this isolation mode is not supported"));
        }
        let Some(original) = self.store.get_task(id).await? else {
            return Ok(false);
        };
        if !matches!(&original.status, TaskStatus::WaitingApproval { reason } if reason == "Manual verification required")
        {
            return Err(anyhow!("Task is not awaiting manual verification"));
        }
        if self.review_revision(&original)? != revision {
            return Err(anyhow!("Review revision is stale"));
        }
        let mut task = original.clone();
        let waiting: Vec<usize> = task
            .subtasks
            .iter()
            .enumerate()
            .filter_map(|(i, step)| {
                matches!(&step.status, TaskStatus::WaitingApproval { reason } if reason == "Manual verification required").then_some(i)
            })
            .collect();
        if waiting.len() != 1 {
            return Err(anyhow!("Expected exactly one step awaiting manual review"));
        }
        let step = &mut task.subtasks[waiting[0]];
        let description = step.expected_verification.clone();
        step.verification_result = Some(agent_domain::VerificationResult {
            passed: approved,
            spec: agent_domain::VerificationSpec::Custom { description },
            details: format!(
                "Manual review {} by claimed reviewer {}",
                if approved { "approved" } else { "rejected" },
                reviewer.trim()
            ),
            executed_at: chrono::Utc::now(),
        });
        step.status = if approved {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed {
                error: "Manual verification rejected".into(),
            }
        };
        task.status = if !approved {
            TaskStatus::Failed {
                error: "Manual verification rejected".into(),
            }
        } else if task
            .subtasks
            .iter()
            .all(|s| s.status == TaskStatus::Completed)
        {
            TaskStatus::Completed
        } else {
            TaskStatus::Queued
        };
        task.updated_at = chrono::Utc::now();
        let engine = if task.status == TaskStatus::Queued {
            let name = task
                .engine_name
                .as_deref()
                .ok_or_else(|| anyhow!("Task has no recorded engine"))?;
            let config = agent_engine::EnginesConfig::load_hierarchical(Some(&self.workspace_dir));
            Some(agent_engine::configured_engine(&config, name, None)?)
        } else {
            None
        };
        let result = TaskResult {
            task_id: id.clone(),
            status: task.status.clone(),
            summary: if approved {
                "Manual verification approved"
            } else {
                "Manual verification rejected"
            }
            .into(),
            artifacts: Vec::new(),
            verifications: task
                .subtasks
                .iter()
                .filter_map(|s| s.verification_result.clone())
                .collect(),
            duration_ms: 0,
            engine_used: task.engine_name.clone(),
        };
        if !self
            .store
            .save_review(
                &original,
                &task,
                &result,
                reviewer.trim(),
                if approved { "approved" } else { "rejected" },
                revision,
            )
            .await?
        {
            return Err(anyhow!("Review revision is stale"));
        }
        if let Some(engine) = engine {
            self.dispatch(id.clone(), engine);
        }
        Ok(true)
    }
}
