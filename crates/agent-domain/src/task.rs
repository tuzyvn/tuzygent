use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TaskError {
    #[error("Subtask '{0}' not found in task")]
    SubtaskNotFound(String),
    #[error("Invalid task operation: {0}")]
    InvalidOperation(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new() -> Self {
        Self(format!("TASK-{}", Uuid::new_v4().simple()))
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Scheduled {
        run_after: chrono::DateTime<chrono::Utc>,
    },
    Queued,
    Dispatched {
        runtime_id: String,
        lease_until: chrono::DateTime<chrono::Utc>,
    },
    PreparingSandbox,
    Executing,
    Supplementing {
        instruction: String,
    },
    WaitingApproval {
        reason: String,
    },
    SelfChecking,
    Completed,
    Failed {
        error: String,
    },
    Canceled {
        reason: String,
    },
    Quarantined {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationSpec {
    Command {
        command: String,
        expected_exit_code: i32,
    },
    FileExists {
        path: String,
        min_bytes: Option<u64>,
    },
    JsonValid {
        path: String,
    },
    ContentContains {
        path: String,
        pattern: String,
    },
    Custom {
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationResult {
    pub passed: bool,
    pub spec: VerificationSpec,
    pub details: String,
    pub executed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: TaskId,
    pub status: TaskStatus,
    pub summary: String,
    pub artifacts: Vec<String>,
    pub verifications: Vec<VerificationResult>,
    pub duration_ms: u128,
    pub engine_used: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubTask {
    pub id: String,
    pub title: String,
    pub command_or_prompt: String,
    pub expected_verification: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub parent_subtask_id: Option<String>,
    #[serde(default)]
    pub remediation_for_error: Option<String>,
    #[serde(default)]
    pub verification_spec: Option<VerificationSpec>,
    #[serde(default)]
    pub verification_result: Option<VerificationResult>,
    #[serde(default)]
    pub output_excerpt: Option<String>,
}

impl SubTask {
    pub fn new(
        id: &str,
        title: &str,
        command_or_prompt: &str,
        expected_verification: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            command_or_prompt: command_or_prompt.to_string(),
            expected_verification: expected_verification.to_string(),
            status: TaskStatus::Queued,
            retry_count: 0,
            parent_subtask_id: None,
            remediation_for_error: None,
            verification_spec: None,
            verification_result: None,
            output_excerpt: None,
        }
    }

    pub fn new_remediation(
        id: &str,
        title: &str,
        command_or_prompt: &str,
        parent_id: &str,
        error_msg: &str,
        retry: u32,
    ) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            command_or_prompt: command_or_prompt.to_string(),
            expected_verification: format!("Remediate and resolve failure from {}", parent_id),
            status: TaskStatus::Queued,
            retry_count: retry,
            parent_subtask_id: Some(parent_id.to_string()),
            remediation_for_error: Some(error_msg.to_string()),
            verification_spec: None,
            verification_result: None,
            output_excerpt: None,
        }
    }

    pub fn is_remediation(&self) -> bool {
        self.parent_subtask_id.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub goal_description: String,
    pub priority: Priority,
    pub status: TaskStatus,
    pub subtasks: Vec<SubTask>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub assigned_agent: Option<crate::identity::AgentId>,
    /// Configured engine name retained for safe recovery of queued tasks.
    #[serde(default)]
    pub engine_name: Option<String>,
    /// Number of consecutive execution or worker failures, used for poison-task isolation and dead-letter queue routing.
    #[serde(default)]
    pub failure_count: usize,
    /// Earliest UTC timestamp at which a delayed task is eligible to transition to Queued and execute.
    #[serde(default)]
    pub run_after: Option<chrono::DateTime<chrono::Utc>>,
    /// Explicit scheduled execution timestamp for delayed tasks.
    #[serde(default)]
    pub scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional recurring cron expression associated with this task.
    #[serde(default)]
    pub cron_expression: Option<String>,
    /// Optional recurring or delayed schedule specification associated with this task.
    #[serde(default)]
    pub schedule_spec: Option<crate::schedule::ScheduleSpec>,
}

impl Task {
    pub fn new(title: &str, goal_description: &str, priority: Priority) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: TaskId::new(),
            title: title.to_string(),
            goal_description: goal_description.to_string(),
            priority,
            status: TaskStatus::Queued,
            subtasks: Vec::new(),
            created_at: now,
            updated_at: now,
            assigned_agent: None,
            engine_name: None,
            failure_count: 0,
            run_after: None,
            scheduled_at: None,
            cron_expression: None,
            schedule_spec: None,
        }
    }

    pub fn new_delayed(
        title: &str,
        goal_description: &str,
        priority: Priority,
        run_after: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let now = chrono::Utc::now();
        let status = if run_after > now {
            TaskStatus::Scheduled { run_after }
        } else {
            TaskStatus::Queued
        };
        Self {
            id: TaskId::new(),
            title: title.to_string(),
            goal_description: goal_description.to_string(),
            priority,
            status,
            subtasks: Vec::new(),
            created_at: now,
            updated_at: now,
            assigned_agent: None,
            engine_name: None,
            failure_count: 0,
            run_after: Some(run_after),
            scheduled_at: Some(run_after),
            cron_expression: None,
            schedule_spec: Some(crate::schedule::ScheduleSpec::OnceAt { run_after }),
        }
    }

    pub fn new_cron(
        title: &str,
        goal_description: &str,
        priority: Priority,
        cron_expression: &str,
    ) -> Result<Self, crate::schedule::ScheduleError> {
        let spec = crate::schedule::ScheduleSpec::cron(cron_expression)?;
        let mut task = Self::new(title, goal_description, priority);
        task.cron_expression = Some(cron_expression.to_string());
        task.schedule_spec = Some(spec);
        Ok(task)
    }

    pub fn with_scheduled_at(mut self, scheduled_at: chrono::DateTime<chrono::Utc>) -> Self {
        self.run_after = Some(scheduled_at);
        self.scheduled_at = Some(scheduled_at);
        self.status = TaskStatus::Scheduled {
            run_after: scheduled_at,
        };
        self.schedule_spec = Some(crate::schedule::ScheduleSpec::OnceAt {
            run_after: scheduled_at,
        });
        self
    }

    pub fn with_cron_expression(mut self, cron_expression: impl Into<String>) -> Self {
        let expr = cron_expression.into();
        self.cron_expression = Some(expr.clone());
        self.schedule_spec = Some(crate::schedule::ScheduleSpec::Cron { expression: expr });
        self
    }

    pub fn add_subtask(&mut self, subtask: SubTask) {
        self.subtasks.push(subtask);
        self.updated_at = chrono::Utc::now();
    }

    pub fn find_subtask(&self, subtask_id: &str) -> Option<&SubTask> {
        self.subtasks.iter().find(|s| s.id == subtask_id)
    }

    pub fn find_subtask_mut(&mut self, subtask_id: &str) -> Option<&mut SubTask> {
        self.subtasks.iter_mut().find(|s| s.id == subtask_id)
    }

    /// Safely retrieve a reference to a subtask by ID, returning `TaskError::SubtaskNotFound`
    /// instead of panicking on None, enabling safe error propagation with the `?` operator.
    pub fn get_subtask(&self, subtask_id: &str) -> Result<&SubTask, TaskError> {
        self.find_subtask(subtask_id)
            .ok_or_else(|| TaskError::SubtaskNotFound(subtask_id.to_string()))
    }

    /// Safely retrieve a mutable reference to a subtask by ID, returning `TaskError::SubtaskNotFound`
    /// instead of panicking on None, enabling safe error propagation with the `?` operator.
    pub fn get_subtask_mut(&mut self, subtask_id: &str) -> Result<&mut SubTask, TaskError> {
        self.find_subtask_mut(subtask_id)
            .ok_or_else(|| TaskError::SubtaskNotFound(subtask_id.to_string()))
    }

    pub fn is_completed(&self) -> bool {
        matches!(self.status, TaskStatus::Completed)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self.status, TaskStatus::Failed { .. })
    }

    pub fn is_quarantined(&self) -> bool {
        matches!(self.status, TaskStatus::Quarantined { .. })
    }

    pub fn is_scheduled(&self) -> bool {
        matches!(self.status, TaskStatus::Scheduled { .. })
    }

    pub fn is_ready_to_execute(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        let due_at = self.scheduled_at.or(self.run_after);
        self.status == TaskStatus::Queued && due_at.is_none_or(|t| t <= now)
    }

    /// Forks this task into a brand new `Task` with an independent `TaskId`, inheriting
    /// checkpoint state (optionally rewound to `from_step`) and optional overridden title/goal.
    pub fn fork_from_checkpoint(
        &self,
        checkpoint: &crate::checkpoint::TaskCheckpoint,
        from_step: Option<usize>,
        new_title: Option<String>,
        new_goal: Option<String>,
    ) -> Result<
        (Self, crate::checkpoint::TaskCheckpoint),
        crate::checkpoint::CheckpointIntegrityError,
    > {
        let new_id = TaskId::new();
        let forked_cp = checkpoint.fork_for_task(new_id.clone(), from_step)?;
        let now = chrono::Utc::now();
        let forked_task = Self {
            id: new_id,
            title: new_title.unwrap_or_else(|| self.title.clone()),
            goal_description: new_goal.unwrap_or_else(|| self.goal_description.clone()),
            priority: self.priority.clone(),
            status: TaskStatus::Queued,
            subtasks: forked_cp.subtasks.clone(),
            created_at: now,
            updated_at: now,
            assigned_agent: self.assigned_agent.clone(),
            engine_name: self.engine_name.clone(),
            failure_count: 0,
            run_after: None,
            scheduled_at: None,
            cron_expression: None,
            schedule_spec: None,
        };
        Ok((forked_task, forked_cp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_lifecycle_and_subtask_management() -> Result<(), TaskError> {
        let mut task = Task::new(
            "Security Hardening",
            "Implement enterprise guardrail policies",
            Priority::High,
        );
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(!task.is_completed());
        assert!(!task.is_failed());
        assert!(task.subtasks.is_empty());

        let subtask1 = SubTask::new(
            "sub-1",
            "Lint security rules",
            "cargo test -p agent-domain",
            "Exit code 0",
        );
        assert!(!subtask1.is_remediation());
        task.add_subtask(subtask1);
        assert_eq!(task.subtasks.len(), 1);

        let remediation = SubTask::new_remediation(
            "sub-1-fix",
            "Remediate test failure",
            "cargo fix --allow-no-vcs",
            "sub-1",
            "Compilation error in guardrail",
            1,
        );
        assert!(remediation.is_remediation());
        assert_eq!(remediation.parent_subtask_id.as_deref(), Some("sub-1"));
        assert_eq!(remediation.retry_count, 1);

        task.add_subtask(remediation);
        assert_eq!(task.subtasks.len(), 2);
        assert!(task.find_subtask("sub-1").is_some());
        assert!(task.find_subtask("sub-1-fix").is_some());
        assert!(task.find_subtask("nonexistent").is_none());

        if let Some(sub) = task.find_subtask_mut("sub-1") {
            sub.status = TaskStatus::Completed;
        }
        assert_eq!(task.get_subtask("sub-1")?.status, TaskStatus::Completed);
        assert_eq!(
            task.find_subtask("sub-1").map(|s| &s.status),
            Some(&TaskStatus::Completed)
        );

        task.status = TaskStatus::Completed;
        assert!(task.is_completed());
        Ok(())
    }

    #[test]
    fn test_safe_subtask_lookup_and_error_handling() {
        let mut task = Task::new(
            "Resilience Check",
            "Verify zero-panic subtask lookup",
            Priority::Normal,
        );

        // Safe query on empty task returns TaskError without panic
        assert_eq!(
            task.get_subtask("missing-id"),
            Err(TaskError::SubtaskNotFound("missing-id".to_string()))
        );
        assert_eq!(
            task.get_subtask_mut("missing-id"),
            Err(TaskError::SubtaskNotFound("missing-id".to_string()))
        );

        // Add subtask and safely access it
        let sub = SubTask::new("st-alpha", "Alpha step", "echo 1", "output 1");
        task.add_subtask(sub);

        assert!(task.get_subtask("st-alpha").is_ok());
        assert_eq!(
            task.get_subtask("st-alpha").map(|s| s.id.as_str()),
            Ok("st-alpha")
        );

        // Error message verification
        match task.get_subtask("nonexistent") {
            Err(err) => assert_eq!(err.to_string(), "Subtask 'nonexistent' not found in task"),
            Ok(_) => unreachable!("Must fail with SubtaskNotFound"),
        }

        let err_op = TaskError::InvalidOperation("Unsupported state transition".to_string());
        assert_eq!(
            err_op.to_string(),
            "Invalid task operation: Unsupported state transition"
        );
    }

    #[test]
    fn test_subtask_status_query_existing_and_nonexistent() {
        let mut task = Task::new(
            "Workflow Status Check",
            "Simulate querying subtask status safely",
            Priority::High,
        );

        let subtask = SubTask::new(
            "sub-valid",
            "Execute unit tests",
            "cargo test",
            "Exit code 0",
        );
        task.add_subtask(subtask);

        // 1. Trường hợp subtask tồn tại:
        // - Kiểm tra trạng thái ban đầu an toàn qua get_subtask
        let status_res = task.get_subtask("sub-valid").map(|s| s.status.clone());
        assert_eq!(status_res, Ok(TaskStatus::Queued));

        // - Cập nhật trạng thái an toàn
        let update_res = task.get_subtask_mut("sub-valid");
        assert!(update_res.is_ok());
        if let Ok(st) = update_res {
            st.status = TaskStatus::Completed;
        }

        // - Xác minh lại trạng thái sau khi cập nhật qua get_subtask và find_subtask mà không unwrap thiếu an toàn
        assert_eq!(
            task.get_subtask("sub-valid").map(|s| &s.status),
            Ok(&TaskStatus::Completed)
        );
        assert_eq!(
            task.find_subtask("sub-valid").map(|s| &s.status),
            Some(&TaskStatus::Completed)
        );

        // 2. Trường hợp subtask không tồn tại:
        // - get_subtask trả về Result Err(TaskError::SubtaskNotFound) thay vì panic
        let missing_id = "sub-nonexistent";
        let missing_res = task.get_subtask(missing_id).map(|s| &s.status);
        assert_eq!(
            missing_res,
            Err(TaskError::SubtaskNotFound(missing_id.to_string()))
        );

        // - get_subtask_mut trả về Err tương ứng
        assert_eq!(
            task.get_subtask_mut(missing_id).map(|s| &s.status),
            Err(TaskError::SubtaskNotFound(missing_id.to_string()))
        );

        // - find_subtask trả về None, xử lý fallback an toàn mà không gây crash/panic
        let status_or_default = task
            .find_subtask(missing_id)
            .map(|s| &s.status)
            .unwrap_or(&TaskStatus::Queued);
        assert_eq!(status_or_default, &TaskStatus::Queued);

        // - Xử lý thông qua pattern matching an toàn với Result
        let is_completed = match task.get_subtask(missing_id) {
            Ok(s) => matches!(s.status, TaskStatus::Completed),
            Err(TaskError::SubtaskNotFound(_)) => false,
            Err(_) => false,
        };
        assert!(!is_completed);
    }

    #[test]
    fn new_delayed_schedules_future_tasks_and_queues_due_tasks() {
        let now = chrono::Utc::now();
        let future = now + chrono::Duration::minutes(5);
        let scheduled = Task::new_delayed("future", "run later", Priority::High, future);

        assert_eq!(
            scheduled.status,
            TaskStatus::Scheduled { run_after: future }
        );
        assert_eq!(scheduled.run_after, Some(future));
        assert_eq!(scheduled.scheduled_at, Some(future));
        assert_eq!(
            scheduled.schedule_spec,
            Some(crate::schedule::ScheduleSpec::OnceAt { run_after: future })
        );

        let past = now - chrono::Duration::minutes(5);
        let queued = Task::new_delayed("due", "run now", Priority::Normal, past);
        assert_eq!(queued.status, TaskStatus::Queued);
        assert_eq!(queued.run_after, Some(past));
        assert_eq!(queued.scheduled_at, Some(past));
        assert_eq!(
            queued.schedule_spec,
            Some(crate::schedule::ScheduleSpec::OnceAt { run_after: past })
        );
    }

    #[test]
    fn new_cron_validates_and_stores_the_schedule() {
        let task = Task::new_cron("daily", "run daily", Priority::Normal, "0 0 * * *")
            .expect("valid cron expression should create a task");
        assert_eq!(task.status, TaskStatus::Queued);
        assert_eq!(task.cron_expression.as_deref(), Some("0 0 * * *"));
        assert_eq!(
            task.schedule_spec,
            Some(crate::schedule::ScheduleSpec::Cron {
                expression: "0 0 * * *".to_string()
            })
        );

        let invalid = Task::new_cron("invalid", "bad schedule", Priority::Low, "not cron");
        assert!(matches!(
            invalid,
            Err(crate::schedule::ScheduleError::InvalidCronExpression { .. })
        ));
    }

    #[test]
    fn with_scheduled_at_sets_schedule_fields_and_scheduled_status() {
        let timestamp = chrono::Utc::now() - chrono::Duration::seconds(1);
        let task =
            Task::new("scheduled", "builder schedule", Priority::High).with_scheduled_at(timestamp);

        assert_eq!(
            task.status,
            TaskStatus::Scheduled {
                run_after: timestamp
            }
        );
        assert_eq!(task.run_after, Some(timestamp));
        assert_eq!(task.scheduled_at, Some(timestamp));
        assert_eq!(
            task.schedule_spec,
            Some(crate::schedule::ScheduleSpec::OnceAt {
                run_after: timestamp
            })
        );
    }

    #[test]
    fn with_cron_expression_stores_expression_without_validating_it() {
        let expression = "this is not parsed as cron";
        let task =
            Task::new("cron", "builder cron", Priority::Normal).with_cron_expression(expression);

        assert_eq!(task.status, TaskStatus::Queued);
        assert_eq!(task.cron_expression.as_deref(), Some(expression));
        assert_eq!(
            task.schedule_spec,
            Some(crate::schedule::ScheduleSpec::Cron {
                expression: expression.to_string()
            })
        );
    }
}
