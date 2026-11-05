use super::AutonomousOodaLoop;
use crate::ooda::context::{
    orient_context, prepare_sandbox_and_brief, publish_event, record_episode_memory,
};
use crate::ooda::execution::QueueExecutionOutcome;
use crate::sandbox::SandboxEnvironment;
use agent_domain::{Task, TaskResult, TaskStatus, VerificationResult};
use std::path::Path;
use tracing::{info, info_span, Instrument};

impl AutonomousOodaLoop {
    async fn handle_task_failure(
        &self,
        task: &mut Task,
        sandbox: &mut SandboxEnvironment,
        _active_workdir: &Path,
        err_msg: &str,
    ) {
        task.status = TaskStatus::Failed {
            error: err_msg.to_string(),
        };
        let _ = sandbox.teardown().await;
        publish_event(
            self.bus.as_ref(),
            "task.failed",
            serde_json::json!({
                "task_id": task.id.0,
                "title": task.title,
                "status": "failed",
                "error": err_msg,
            }),
        );
    }

    pub async fn run_goal(&self, task: &mut Task) -> Result<TaskStatus, String> {
        let res = self.run_goal_detailed(task).await?;
        Ok(res.status)
    }

    pub async fn run_goal_detailed(&self, task: &mut Task) -> Result<TaskResult, String> {
        let start_time = std::time::Instant::now();
        let mut all_verifications: Vec<VerificationResult> = task.subtasks.iter()
            .filter(|step| step.status == TaskStatus::Completed)
            .filter_map(|step| step.verification_result.clone()).collect();

        info!("🎯 [OODA] Starting autonomous run for goal: {}", task.title);
        task.status = TaskStatus::PreparingSandbox;

        publish_event(
            self.bus.as_ref(),
            "task.started",
            serde_json::json!({
                "task_id": task.id.0,
                "title": task.title,
                "goal": task.goal_description,
            }),
        );

        // 0. Observe: Chuẩn bị Sandbox và tiêm Agent Brief Context theo chuẩn Multica
        let observe_span = info_span!(
            "ooda_phase",
            task_id = %task.id.0,
            subtask_id = "",
            execution_phase = super::OODA_PHASE_OBSERVE,
        );
        let (mut sandbox, active_workdir) = match prepare_sandbox_and_brief(
            &self.workspace_dir,
            &self.sandbox_config,
            task,
        )
        .instrument(observe_span)
        .await
        {
            Ok(res) => res,
            Err(err_msg) => {
                task.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                publish_event(
                    self.bus.as_ref(),
                    "task.failed",
                    serde_json::json!({
                        "task_id": task.id.0,
                        "title": task.title,
                        "error": err_msg,
                    }),
                );
                return Err(err_msg);
            }
        };

        // 1. Orient: Load Relevant Long-term Context
        let orient_span = info_span!(
            "ooda_phase",
            task_id = %task.id.0,
            subtask_id = "",
            execution_phase = super::OODA_PHASE_ORIENT,
        );
        let (memory, long_term_ctx) =
            match orient_context(&self.workspace_dir, &self.persona, task)
                .instrument(orient_span)
                .await
            {
                Ok(res) => res,
                Err(err_msg) => {
                    self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_msg)
                        .await;
                    return Err(err_msg);
                }
            };
        let mut working_context = String::new();
        for step in task.subtasks.iter().filter(|step| step.status == TaskStatus::Completed) {
            if let Some(output) = &step.output_excerpt {
                crate::ooda::execution::runner::update_working_context(&mut working_context, &step.id, output);
            }
        }

        // 2. Decide: Decompose Goal into SubTasks Dynamically
        let decide_span = info_span!(
            "ooda_phase",
            task_id = %task.id.0,
            subtask_id = "",
            execution_phase = super::OODA_PHASE_DECIDE,
        );
        if let Err(e) = self
            .scratchpad
            .append_log(&format!("Starting Goal: {}", task.title))
            .await
        {
            let err_msg = format!("Failed to append scratchpad log: {}", e);
            self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_msg)
                .await;
            return Err(err_msg);
        }

        if task.subtasks.is_empty() {
            let decomposer = self.decomposer();
            let dynamic_subtasks = tokio::select! {
                biased;
                _ = super::wait_for_cancellation(&self.cancellation_token) => {
                    let reason = "Canceled by user request".to_string();
                    task.status = TaskStatus::Canceled { reason: reason.clone() };
                    let _ = sandbox.teardown().await;
                    return Ok(TaskResult { task_id: task.id.clone(), status: task.status.clone(), summary: reason,
                        artifacts: Vec::new(), verifications: all_verifications, duration_ms: start_time.elapsed().as_millis(),
                        engine_used: Some(self.engine.engine_kind().to_string()) });
                }
                result = decomposer.decompose_goal(
                    &task.title,
                    &task.goal_description,
                    &active_workdir,
                    &long_term_ctx,
                ).instrument(decide_span.clone()) => result,
            };
            task.subtasks = dynamic_subtasks;
        }

        if task.subtasks.is_empty() {
            let err_msg = format!(
                "No subtasks generated or available for goal '{}'",
                task.title
            );
            self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_msg)
                .await;
            return Err(err_msg);
        }

        task.status = TaskStatus::Executing;

        // 3. Act & Re-Act: Hàng đợi thực thi thích ứng (Queue with Dynamic Backtracking)
        let act_span = info_span!(
            "ooda_phase",
            task_id = %task.id.0,
            subtask_id = "",
            execution_phase = super::OODA_PHASE_ACT,
        );
        let executor = self.executor();
        if let Err(err) = executor.checkpoint(task).await {
            self.handle_task_failure(task, &mut sandbox, &active_workdir, &err).await;
            return Err(err);
        }
        let outcome = match executor
            .execute_queue(
                task,
                &active_workdir,
                &long_term_ctx,
                &mut all_verifications,
                &mut working_context,
            )
            .instrument(act_span)
            .await
        {
            Ok(outcome) => outcome,
            Err(err_str) => {
                self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_str)
                    .await;
                return Err(err_str);
            }
        };

        if let QueueExecutionOutcome::BlockedWaitingApproval { reason } = outcome {
            return Ok(TaskResult {
                task_id: task.id.clone(),
                status: task.status.clone(),
                summary: reason,
                artifacts: Vec::new(),
                verifications: all_verifications,
                duration_ms: start_time.elapsed().as_millis(),
                engine_used: Some(self.engine.engine_kind().to_string()),
            });
        }

        if let QueueExecutionOutcome::Canceled { reason } = outcome {
            let _ = sandbox.teardown().await;
            publish_event(
                self.bus.as_ref(),
                "task.canceled",
                serde_json::json!({
                    "task_id": task.id.0,
                    "title": task.title,
                    "reason": reason,
                }),
            );
            return Ok(TaskResult {
                task_id: task.id.clone(),
                status: task.status.clone(),
                summary: reason,
                artifacts: Vec::new(),
                verifications: all_verifications,
                duration_ms: start_time.elapsed().as_millis(),
                engine_used: Some(self.engine.engine_kind().to_string()),
            });
        }

        let _ = sandbox.teardown().await;

        let mut had_fatal_failure = matches!(outcome, QueueExecutionOutcome::Failed { .. });
        if !had_fatal_failure
            && task
                .subtasks
                .iter()
                .any(|st| st.status != TaskStatus::Completed)
        {
            had_fatal_failure = true;
            task.status = TaskStatus::Failed {
                error: "Some subtasks did not complete".into(),
            };
        }

        if !had_fatal_failure {
            task.status = TaskStatus::Completed;
            info!(
                "🎉 [OODA] Entire Goal '{}' completed successfully!",
                task.title
            );
            publish_event(
                self.bus.as_ref(),
                "task.completed",
                serde_json::json!({
                    "task_id": task.id.0,
                    "title": task.title,
                    "status": "completed",
                }),
            );
        } else {
            let error_detail = match &task.status {
                TaskStatus::Failed { error } => error.clone(),
                _ => match &outcome {
                    QueueExecutionOutcome::Failed { error } => error.clone(),
                    _ => "Subtask execution failed".to_string(),
                },
            };
            publish_event(
                self.bus.as_ref(),
                "task.failed",
                serde_json::json!({
                    "task_id": task.id.0,
                    "title": task.title,
                    "status": "failed",
                    "error": error_detail,
                }),
            );
        }

        // Record episode memory with factual verification indicator
        let all_ok = !had_fatal_failure;
        record_episode_memory(
            memory,
            task.id.0.clone(),
            task,
            &working_context,
            &all_verifications,
            all_ok,
        )
        .await;

        let duration_ms = start_time.elapsed().as_millis();
        let summary = if had_fatal_failure {
            let err_desc = match &task.status {
                TaskStatus::Failed { error } => error.as_str(),
                _ => match &outcome {
                    QueueExecutionOutcome::Failed { error } => error.as_str(),
                    _ => "Execution failed",
                },
            };
            format!("Goal '{}' failed: {}", task.title, err_desc)
        } else {
            format!(
                "Goal '{}' finished with status {:?}. Total {} subtasks processed.",
                task.title, task.status, task.subtasks.len()
            )
        };
        let task_result = TaskResult {
            task_id: task.id.clone(),
            status: task.status.clone(),
            summary,
            artifacts: Vec::new(),
            verifications: all_verifications,
            duration_ms,
            engine_used: Some(self.engine.engine_kind().to_string()),
        };

        Ok(task_result)
    }
}
