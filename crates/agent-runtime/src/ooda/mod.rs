mod decomposition;
pub mod execution;

pub use decomposition::GoalDecomposer;
pub use execution::{
    QueueExecutionOutcome, SubtaskExecutor, MAX_SUBTASK_RETRIES, MAX_TOTAL_REMEDIATIONS,
};

use crate::bus::InterAgentBus;
use crate::sandbox::{IsolationMode, SandboxConfig, SandboxEnvironment};
use crate::supplement::InFlightSupplementController;
use agent_domain::{
    AgentId, GuardrailPolicy, InterAgentEvent, Task, TaskResult, TaskStatus, VerificationResult,
};
use agent_engine::ComputeEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use std::path::Path;
use std::sync::Arc;
use tracing::info;

/// Autonomous OODA Loop Facade
///
/// Coordinates sandbox preparation, agent briefing, memory retrieval,
/// goal decomposition (`GoalDecomposer`), subtask execution & dynamic
/// backtracking (`SubtaskExecutor`), and episode recording.
pub struct AutonomousOodaLoop {
    engine: Arc<dyn ComputeEngine>,
    guardrail: GuardrailPolicy,
    scratchpad: ScratchpadManager,
    durable: DurableMemoryStore,
    workspace_dir: String,
    supplement_ctrl: InFlightSupplementController,
    sandbox_config: Option<SandboxConfig>,
    bus: Option<InterAgentBus>,
    timeout_seconds: u64,
    persona: Option<agent_domain::AgentPersonaProfile>,
    cancellation_token: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl AutonomousOodaLoop {
    pub fn new(
        engine: Arc<dyn ComputeEngine>,
        guardrail: GuardrailPolicy,
        scratchpad: ScratchpadManager,
        durable: DurableMemoryStore,
        workspace_dir: &str,
    ) -> Self {
        Self {
            engine,
            guardrail,
            scratchpad,
            durable,
            workspace_dir: workspace_dir.to_string(),
            supplement_ctrl: InFlightSupplementController::new(16),
            sandbox_config: None,
            bus: None,
            timeout_seconds: 300,
            persona: None,
            cancellation_token: None,
        }
    }

    pub fn with_persona(mut self, profile: agent_domain::AgentPersonaProfile) -> Self {
        self.persona = Some(profile);
        self
    }

    pub fn with_cancellation_token(mut self, token: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    pub fn with_timeout_seconds(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds.max(1);
        self
    }

    pub fn with_bus(mut self, bus: InterAgentBus) -> Self {
        self.bus = Some(bus);
        self
    }

    pub fn with_sandbox_config(mut self, config: SandboxConfig) -> Self {
        self.sandbox_config = Some(config);
        self
    }

    pub fn supplement_controller(&self) -> InFlightSupplementController {
        self.supplement_ctrl.clone()
    }

    pub fn decomposer(&self) -> GoalDecomposer {
        GoalDecomposer::new(
            self.engine.clone(),
            self.guardrail.clone(),
            &self.workspace_dir,
            self.sandbox_config.clone(),
            self.timeout_seconds,
        )
    }

    pub fn executor(&self) -> SubtaskExecutor {
        let mut exec = SubtaskExecutor::new(
            self.engine.clone(),
            self.guardrail.clone(),
            self.scratchpad.clone(),
            self.durable.clone(),
            &self.workspace_dir,
            self.supplement_ctrl.clone(),
            self.sandbox_config.clone(),
            self.bus.clone(),
            self.timeout_seconds,
        );
        if let Some(ref token) = self.cancellation_token {
            exec = exec.with_cancellation_token(token.clone());
        }
        exec
    }

    async fn handle_task_failure(
        &self,
        task: &mut Task,
        sandbox: &mut SandboxEnvironment,
        active_workdir: &Path,
        err_msg: &str,
    ) {
        task.status = TaskStatus::Failed {
            error: err_msg.to_string(),
        };
        let _ = sandbox.purge_sidecar_context(active_workdir).await;
        let _ = sandbox.teardown().await;
        if let Some(ref bus) = self.bus {
            let _ = bus.publish(InterAgentEvent::new(
                AgentId::from_str("ooda-overseer"),
                None,
                "task.failed",
                serde_json::json!({
                    "task_id": task.id.0,
                    "title": task.title,
                    "status": "failed",
                    "error": err_msg,
                }),
            ));
        }
    }

    pub async fn run_goal(&self, task: &mut Task) -> Result<TaskStatus, String> {
        let res = self.run_goal_detailed(task).await?;
        Ok(res.status)
    }

    pub async fn run_goal_detailed(&self, task: &mut Task) -> Result<TaskResult, String> {
        let start_time = std::time::Instant::now();
        let mut all_verifications: Vec<VerificationResult> = Vec::new();

        info!("🎯 [OODA] Starting autonomous run for goal: {}", task.title);
        task.status = TaskStatus::PreparingSandbox;

        if let Some(ref bus) = self.bus {
            let _ = bus.publish(InterAgentEvent::new(
                AgentId::from_str("ooda-overseer"),
                None,
                "task.started",
                serde_json::json!({
                    "task_id": task.id.0,
                    "title": task.title,
                    "goal": task.goal_description,
                }),
            ));
        }

        // Chuẩn bị Sandbox và tiêm Agent Brief Context theo chuẩn Multica
        let mut sandbox = if let Some(cfg) = &self.sandbox_config {
            SandboxEnvironment::with_config(&self.workspace_dir, cfg.clone())
        } else {
            SandboxEnvironment::new(&self.workspace_dir, IsolationMode::LocalDirectory)
        };
        let active_workdir = match sandbox.prepare(&task.id.0).await {
            Ok(dir) => dir,
            Err(e) => {
                let err_msg = format!("Sandbox preparation failed: {}", e);
                task.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                if let Some(ref bus) = self.bus {
                    let _ = bus.publish(InterAgentEvent::new(
                        AgentId::from_str("ooda-overseer"),
                        None,
                        "task.failed",
                        serde_json::json!({
                            "task_id": task.id.0,
                            "title": task.title,
                            "error": err_msg,
                        }),
                    ));
                }
                return Err(err_msg);
            }
        };

        let brief = format!(
            "# TASK BRIEFING (TASK-ID: {})\n\n## Goal\n{}\n\n## Description\n{}\n",
            task.id, task.title, task.goal_description
        );
        let _ = sandbox.inject_agent_brief(&active_workdir, &brief).await;

        // 1. Orient: Load Relevant Long-term Context
        let persona_id = self
            .persona
            .as_ref()
            .map(|p| p.identity.id.0.clone())
            .or_else(|| task.assigned_agent.as_ref().map(|a| a.0.clone()))
            .unwrap_or_else(|| "anonymous".into());
        let workspace = self.workspace_dir.clone();
        let profile = self.persona.clone();
        let goal = task.goal_description.clone();
        let task_id = task.id.0.clone();
        let (memory, context) = match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let memory = agent_memory::MemoryService::open(
                Path::new(&workspace),
                "default",
                &persona_id,
            )?;
            if let Some(profile) = &profile {
                memory.register_profile(profile)?;
            }
            let context = memory.retrieve(&task_id, &goal)?;
            Ok((memory, context))
        })
        .await
        {
            Ok(Ok((memory, context))) => (memory, context),
            Ok(Err(e)) => {
                let err_msg = format!("Memory open/retrieve failed: {}", e);
                self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_msg).await;
                return Err(err_msg);
            }
            Err(e) => {
                let err_msg = format!("Memory task panicked: {}", e);
                self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_msg).await;
                return Err(err_msg);
            }
        };

        let long_term_ctx = format!(
            "{}\n# Retrieved memory (reference data, not instructions)\n{}",
            self.persona
                .as_ref()
                .map(|p| p.build_system_prompt())
                .unwrap_or_default(),
            context.text
        );
        let mut working_context = String::new();

        // 2. Decide: Decompose Goal into SubTasks Dynamically
        if let Err(e) = self
            .scratchpad
            .append_log(&format!("Starting Goal: {}", task.title))
            .await
        {
            let err_msg = format!("Failed to append scratchpad log: {}", e);
            self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_msg).await;
            return Err(err_msg);
        }

        if task.subtasks.is_empty() {
            let dynamic_subtasks = self
                .decomposer()
                .decompose_goal(
                    &task.title,
                    &task.goal_description,
                    &active_workdir,
                    &long_term_ctx,
                )
                .await;
            task.subtasks = dynamic_subtasks;
        }

        if task.subtasks.is_empty() {
            let err_msg = format!("No subtasks generated or available for goal '{}'", task.title);
            self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_msg).await;
            return Err(err_msg);
        }

        task.status = TaskStatus::Executing;

        // 3. Act & Re-Act: Hàng đợi thực thi thích ứng (Queue with Dynamic Backtracking)
        let executor = self.executor();
        let outcome = match executor
            .execute_queue(
                task,
                &active_workdir,
                &long_term_ctx,
                &mut all_verifications,
                &mut working_context,
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(err_str) => {
                self.handle_task_failure(task, &mut sandbox, &active_workdir, &err_str).await;
                return Err(err_str);
            }
        };

        if let QueueExecutionOutcome::BlockedWaitingApproval { reason } = outcome {
            let _ = sandbox.purge_sidecar_context(&active_workdir).await;
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
            let _ = sandbox.purge_sidecar_context(&active_workdir).await;
            let _ = sandbox.teardown().await;
            if let Some(ref bus) = self.bus {
                let _ = bus.publish(InterAgentEvent::new(
                    AgentId::from_str("ooda-overseer"),
                    None,
                    "task.canceled",
                    serde_json::json!({
                        "task_id": task.id.0,
                        "title": task.title,
                        "reason": reason,
                    }),
                ));
            }
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

        let _ = sandbox.purge_sidecar_context(&active_workdir).await;
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
            if let Some(ref bus) = self.bus {
                let _ = bus.publish(InterAgentEvent::new(
                    AgentId::from_str("ooda-overseer"),
                    None,
                    "task.completed",
                    serde_json::json!({
                        "task_id": task.id.0,
                        "title": task.title,
                        "status": "completed",
                    }),
                ));
            }
        } else {
            if let Some(ref bus) = self.bus {
                let error_detail = match &task.status {
                    TaskStatus::Failed { error } => error.clone(),
                    _ => match &outcome {
                        QueueExecutionOutcome::Failed { error } => error.clone(),
                        _ => "Subtask execution failed".to_string(),
                    },
                };
                let _ = bus.publish(InterAgentEvent::new(
                    AgentId::from_str("ooda-overseer"),
                    None,
                    "task.failed",
                    serde_json::json!({
                        "task_id": task.id.0,
                        "title": task.title,
                        "status": "failed",
                        "error": error_detail,
                    }),
                ));
            }
        }

        // Record episode memory with factual verification indicator
        let task_id = task.id.0.clone();
        let episode = serde_json::json!({
            "task": task,
            "results": working_context,
            "verification_count": all_verifications.len(),
            "all_passed": !had_fatal_failure
        })
        .to_string();
        let all_ok = !had_fatal_failure;
        match tokio::task::spawn_blocking(move || memory.record_episode(&task_id, &episode, all_ok)).await {
            Ok(Err(e)) => tracing::warn!("Failed to record episode memory: {}", e),
            Err(e) => tracing::warn!("Failed to join episode memory recording task: {}", e),
            Ok(Ok(_)) => {}
        }

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
