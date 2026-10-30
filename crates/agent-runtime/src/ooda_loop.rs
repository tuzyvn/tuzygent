use crate::bus::InterAgentBus;
use crate::sandbox::{IsolationMode, SandboxConfig, SandboxEnvironment};
use crate::supplement::InFlightSupplementController;
use agent_domain::{AgentId, GuardrailPolicy, InterAgentEvent, SubTask, Task, TaskStatus};
use agent_engine::{ComputeEngine, EngineInput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

const MAX_SUBTASK_RETRIES: u32 = 3;
const MAX_TOTAL_REMEDIATIONS: usize = 6;

pub struct AutonomousOodaLoop {
    engine: Arc<dyn ComputeEngine>,
    guardrail: GuardrailPolicy,
    scratchpad: ScratchpadManager,
    durable: DurableMemoryStore,
    workspace_dir: String,
    supplement_ctrl: InFlightSupplementController,
    sandbox_config: Option<SandboxConfig>,
    bus: Option<InterAgentBus>,
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
        }
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

    /// Tự động phân rã Goal thành cây SubTasks động qua ComputeEngine (không gán cứng)
    async fn decompose_goal_dynamically(
        &self,
        task_title: &str,
        goal_description: &str,
        workdir: &Path,
        system_context: &str,
    ) -> Vec<SubTask> {
        info!("🧠 [OODA Planner] Phân tích mục tiêu và phân rã Subtasks động qua LLM...");

        let planning_prompt = format!(
            "You are an Autonomous AI System Architect and Planner.\n\
            Goal Title: {title}\n\
            Goal Description: {desc}\n\
            Workspace Directory: {workdir}\n\n\
            Analyze the requirement and decompose it into 2 to 5 concrete, actionable, sequential subtasks.\n\
            For each subtask, specify:\n\
            - \"id\": Unique ID (e.g. \"ST-1\", \"ST-2\")\n\
            - \"title\": Brief action title\n\
            - \"command_or_prompt\": Exact command or concrete instruction to execute\n\
            - \"expected_verification\": Specific condition or metric to verify completion\n\n\
            Output ONLY a valid JSON array of objects conforming to the schema above. Do not include markdown preamble or conversational text.",
            title = task_title,
            desc = goal_description,
            workdir = workdir.display(),
        );

        let input = EngineInput {
            prompt_or_command: planning_prompt,
            system_context: system_context.to_string(),
            working_dir: workdir.to_string_lossy().to_string(),
            timeout_seconds: 60,
            on_stream_chunk: None,
        };

        if let Ok(out) = self.engine.execute(&input).await {
            let raw = out.stdout_content.trim();
            if let Some(parsed) = Self::parse_subtasks_json(raw) {
                if !parsed.is_empty() {
                    info!(
                        "📋 [OODA Planner] Đã tự phân rã thành công {} Subtasks động.",
                        parsed.len()
                    );
                    return parsed;
                }
            }
        }

        // Fallback tự sinh dựa trên mục tiêu thực tế (không dùng chuỗi mẫu gán cứng)
        warn!("⚠️ [OODA Planner] Không phân giải được JSON phân rã từ LLM, sử dụng tác vụ thực tế trực tiếp");
        vec![SubTask::new(
            "ST-1",
            task_title,
            goal_description,
            "Execution completed with zero errors",
        )]
    }

    /// Trích xuất và parse mảng JSON SubTask từ văn bản LLM
    fn parse_subtasks_json(raw: &str) -> Option<Vec<SubTask>> {
        let json_text = if let Some(start) = raw.find("```json") {
            let after = &raw[start + 7..];
            if let Some(end) = after.find("```") {
                after[..end].trim()
            } else {
                after.trim()
            }
        } else if let Some(start) = raw.find("```") {
            let after = &raw[start + 3..];
            if let Some(end) = after.find("```") {
                after[..end].trim()
            } else {
                after.trim()
            }
        } else if let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) {
            if end >= start {
                &raw[start..=end]
            } else {
                raw
            }
        } else {
            raw
        };

        let val: Value = serde_json::from_str(json_text).ok()?;
        let arr = val.as_array()?;
        let mut subtasks = Vec::new();

        for (idx, item) in arr.iter().enumerate() {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(&format!("ST-{}", idx + 1))
                .to_string();
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Execute subtask")
                .to_string();
            let command_or_prompt = item
                .get("command_or_prompt")
                .or_else(|| item.get("command"))
                .or_else(|| item.get("prompt"))
                .and_then(|v| v.as_str())
                .unwrap_or(&title)
                .to_string();
            let expected_verification = item
                .get("expected_verification")
                .or_else(|| item.get("verification"))
                .and_then(|v| v.as_str())
                .unwrap_or("Status code 0")
                .to_string();

            subtasks.push(SubTask::new(
                &id,
                &title,
                &command_or_prompt,
                &expected_verification,
            ));
        }

        Some(subtasks)
    }

    /// Tự động sinh Remediation SubTask khi gặp lỗi để tự sửa sai (Backtracking Re-Act)
    async fn generate_remediation_subtask(
        &self,
        task_title: &str,
        failed_subtask: &SubTask,
        error_context: &str,
        workdir: &Path,
        system_context: &str,
        retry_num: u32,
    ) -> Option<SubTask> {
        info!(
            "🛠️ [OODA Self-Healing] Đang phân tích nguyên nhân lỗi để sinh bước khắc phục (Lần thử {}/{})...",
            retry_num, MAX_SUBTASK_RETRIES
        );

        let remediation_prompt = format!(
            "You are an Autonomous AI Problem Solver.\n\
            Overall Goal: {goal}\n\
            Failed Subtask ID: {subtask_id}\n\
            Failed Subtask Title: {subtask_title}\n\
            Failed Instruction: {cmd}\n\
            Error / Failure Details:\n{err}\n\n\
            Expected Verification: {verification}\n\n\
            Analyze why this step failed and formulate a concrete Remediation Subtask to fix the root cause.\n\
            Output ONLY a JSON object with keys:\n\
            - \"id\": \"{subtask_id}-FIX-{retry}\"\n\
            - \"title\": \"Remediation: concise fix title\"\n\
            - \"command_or_prompt\": \"specific corrective command or prompt\"\n\
            - \"expected_verification\": \"condition proving the fix worked\"\n\
            Do not include markdown preamble outside the json object.",
            goal = task_title,
            subtask_id = failed_subtask.id,
            subtask_title = failed_subtask.title,
            cmd = failed_subtask.command_or_prompt,
            err = error_context,
            verification = failed_subtask.expected_verification,
            retry = retry_num,
        );

        let input = EngineInput {
            prompt_or_command: remediation_prompt,
            system_context: system_context.to_string(),
            working_dir: workdir.to_string_lossy().to_string(),
            timeout_seconds: 60,
            on_stream_chunk: None,
        };

        if let Ok(out) = self.engine.execute(&input).await {
            let raw = out.stdout_content.trim();
            let json_text = if let Some(start) = raw.find("```json") {
                let after = &raw[start + 7..];
                after
                    .find("```")
                    .map(|end| &after[..end])
                    .unwrap_or(after)
                    .trim()
            } else if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) {
                if end >= start {
                    &raw[start..=end]
                } else {
                    raw
                }
            } else {
                raw
            };

            if let Ok(val) = serde_json::from_str::<Value>(json_text) {
                let default_id = format!("{}-FIX-{}", failed_subtask.id, retry_num);
                let id = val
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_id);
                let title = val
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Remediate error");
                let cmd = val
                    .get("command_or_prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Apply remediation");
                let ver = val
                    .get("expected_verification")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Error resolved");

                let mut rem = SubTask::new_remediation(
                    id,
                    title,
                    cmd,
                    &failed_subtask.id,
                    error_context,
                    retry_num,
                );
                rem.expected_verification = ver.to_string();
                info!(
                    "✨ [OODA Self-Healing] Đã tạo thành công Remediation SubTask: {} ({})",
                    rem.title, rem.id
                );
                return Some(rem);
            }
        }

        None
    }

    pub async fn run_goal(&self, task: &mut Task) -> Result<TaskStatus, String> {
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
        let active_workdir = sandbox
            .prepare(&task.id.0)
            .await
            .map_err(|e| e.to_string())?;

        let brief = format!(
            "# TASK BRIEFING (TASK-ID: {})\n\n## Goal\n{}\n\n## Description\n{}\n",
            task.id, task.title, task.goal_description
        );
        let _ = sandbox.inject_agent_brief(&active_workdir, &brief).await;

        // 1. Orient: Load Relevant Long-term Context
        let long_term_ctx = self
            .durable
            .load_relevant_context(&task.goal_description)
            .await
            .unwrap_or_default();

        // 2. Decide: Decompose Goal into SubTasks Dynamically
        self.scratchpad
            .append_log(&format!("Starting Goal: {}", task.title))
            .await
            .map_err(|e| e.to_string())?;

        if task.subtasks.is_empty() {
            let dynamic_subtasks = self
                .decompose_goal_dynamically(
                    &task.title,
                    &task.goal_description,
                    &active_workdir,
                    &long_term_ctx,
                )
                .await;
            task.subtasks = dynamic_subtasks;
        }

        task.status = TaskStatus::Executing;

        // 3. Act & Re-Act: Hàng đợi thực thi thích ứng (Queue with Dynamic Backtracking)
        let mut subtask_index = 0;
        let mut total_remediations_spawned = 0;
        let mut had_fatal_failure = false;

        while subtask_index < task.subtasks.len() {
            let mut subtask = task.subtasks[subtask_index].clone();
            info!(
                "⚙️ [OODA] [{}/{}] Executing: {} ({})",
                subtask_index + 1,
                task.subtasks.len(),
                subtask.title,
                subtask.id
            );

            // Safety Guardrail Check
            if let Err(red_line_err) = self
                .guardrail
                .check_command_safety(&subtask.command_or_prompt)
            {
                warn!("🚨 [GUARDRAIL BLOCK]: {}", red_line_err);
                subtask.status = TaskStatus::WaitingApproval {
                    reason: red_line_err.clone(),
                };
                task.status = TaskStatus::WaitingApproval {
                    reason: red_line_err.clone(),
                };
                task.subtasks[subtask_index] = subtask;
                let _ = self
                    .durable
                    .save_lesson(&format!(
                        "Guardrail blocked command in task '{}': {}",
                        task.title, red_line_err
                    ))
                    .await;
                return Ok(task.status.clone());
            }

            subtask.status = TaskStatus::Executing;
            task.subtasks[subtask_index] = subtask.clone();

            if let Some(ref bus) = self.bus {
                let _ = bus.publish(InterAgentEvent::new(
                    AgentId::from_str("ooda-overseer"),
                    None,
                    "task.subtask_executing",
                    serde_json::json!({
                        "task_id": task.id.0,
                        "subtask_id": subtask.id,
                        "title": subtask.title,
                        "command": subtask.command_or_prompt,
                    }),
                ));
            }

            // Kiểm tra chỉ thị bổ sung in-flight
            let mut active_system_context = long_term_ctx.clone();
            if let Some(sup) = self.supplement_ctrl.try_recv().await {
                info!(
                    "💡 [OODA] Incorporating In-Flight Supplement from: {}",
                    sup.author_name
                );
                active_system_context.push_str("\n\n");
                active_system_context.push_str(&sup.to_instruction_block());
                let _ = self
                    .durable
                    .save_user_directive(&format!(
                        "Directive from {}: {}",
                        sup.author_name, sup.content
                    ))
                    .await;
            }

            let stream_cb = if let Some(ref bus) = self.bus {
                let bus_clone = bus.clone();
                let task_id_str = task.id.0.clone();
                let subtask_id_str = subtask.id.clone();
                Some(Arc::new(move |chunk: &str| {
                    let evt = InterAgentEvent::new(
                        AgentId::from_str("ooda-overseer"),
                        None,
                        "task.stream_delta",
                        serde_json::json!({
                            "task_id": task_id_str,
                            "subtask_id": subtask_id_str,
                            "delta": chunk,
                        }),
                    );
                    let _ = bus_clone.publish(evt);
                }) as agent_engine::StreamCallback)
            } else {
                None
            };

            let engine_input = EngineInput {
                prompt_or_command: subtask.command_or_prompt.clone(),
                system_context: active_system_context.clone(),
                working_dir: active_workdir.to_string_lossy().to_string(),
                timeout_seconds: 300,
                on_stream_chunk: stream_cb,
            };

            let execution_result = self.engine.execute(&engine_input).await;

            // 4. Self-Check & Dynamic Backtracking
            match execution_result {
                Ok(output) if output.exit_code == 0 => {
                    subtask.status = TaskStatus::Completed;
                    info!(
                        "✅ [OODA] SubTask {} verified and completed (exit 0).",
                        subtask.id
                    );

                    if let Some(diff) = Self::capture_git_diff(&active_workdir).await {
                        info!("📝 [OODA Git Diff Tracking]:\n{}", diff);
                        if let Some(ref bus) = self.bus {
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
                        let _ = self
                            .scratchpad
                            .append_log(&format!(
                                "Git changes detected after {}:\n{}",
                                subtask.id, diff
                            ))
                            .await;
                    }

                    self.scratchpad
                        .append_log(&format!(
                            "SubTask {} completed successfully in {}ms",
                            subtask.id, output.execution_duration_ms
                        ))
                        .await
                        .map_err(|e| e.to_string())?;

                    task.subtasks[subtask_index] = subtask;
                    subtask_index += 1;
                }
                Ok(output) => {
                    let err_msg = format!(
                        "Exit code non-zero: {} | stdout: {}",
                        output.exit_code,
                        output.stdout_content.trim()
                    );
                    warn!("⚠️ [OODA] SubTask {} failed: {}", subtask.id, err_msg);

                    if subtask.retry_count < MAX_SUBTASK_RETRIES
                        && total_remediations_spawned < MAX_TOTAL_REMEDIATIONS
                    {
                        subtask.retry_count += 1;
                        total_remediations_spawned += 1;
                        task.subtasks[subtask_index] = subtask.clone();

                        let remediation = self
                            .generate_remediation_subtask(
                                &task.title,
                                &subtask,
                                &err_msg,
                                &active_workdir,
                                &active_system_context,
                                subtask.retry_count,
                            )
                            .await;

                        if let Some(rem_task) = remediation {
                            let _ = self
                                .durable
                                .save_lesson(&format!(
                                    "Self-Correction in '{}': Formulated step '{}' to fix {}",
                                    task.title, rem_task.title, err_msg
                                ))
                                .await;
                            task.subtasks.insert(subtask_index + 1, rem_task);
                            subtask_index += 1;
                            continue;
                        }
                    }

                    // Hết lượt thử lại hoặc không sinh được remediation
                    subtask.status = TaskStatus::Failed {
                        error: err_msg.clone(),
                    };
                    task.subtasks[subtask_index] = subtask;
                    task.status = TaskStatus::Failed {
                        error: err_msg.clone(),
                    };
                    had_fatal_failure = true;
                    let _ = self
                        .durable
                        .save_lesson(&format!(
                            "Exhausted retries in subtask '{}' of task '{}': {}",
                            task.subtasks[subtask_index].title, task.title, err_msg
                        ))
                        .await;
                    break;
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    error!("❌ [OODA] SubTask execution error: {}", err_msg);

                    if subtask.retry_count < MAX_SUBTASK_RETRIES
                        && total_remediations_spawned < MAX_TOTAL_REMEDIATIONS
                    {
                        subtask.retry_count += 1;
                        total_remediations_spawned += 1;
                        task.subtasks[subtask_index] = subtask.clone();

                        let remediation = self
                            .generate_remediation_subtask(
                                &task.title,
                                &subtask,
                                &err_msg,
                                &active_workdir,
                                &active_system_context,
                                subtask.retry_count,
                            )
                            .await;

                        if let Some(rem_task) = remediation {
                            let _ = self
                                .durable
                                .save_lesson(&format!(
                                    "Self-Correction in '{}': Formulated step '{}' to fix {}",
                                    task.title, rem_task.title, err_msg
                                ))
                                .await;
                            task.subtasks.insert(subtask_index + 1, rem_task);
                            subtask_index += 1;
                            continue;
                        }
                    }

                    subtask.status = TaskStatus::Failed {
                        error: err_msg.clone(),
                    };
                    task.subtasks[subtask_index] = subtask;
                    task.status = TaskStatus::Failed {
                        error: err_msg.clone(),
                    };
                    had_fatal_failure = true;
                    let _ = self
                        .durable
                        .save_lesson(&format!(
                            "Execution failure in '{}' of task '{}': {}",
                            task.subtasks[subtask_index].title, task.title, err_msg
                        ))
                        .await;
                    break;
                }
            }
        }

        let _ = sandbox.purge_sidecar_context(&active_workdir).await;

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
                let _ = bus.publish(InterAgentEvent::new(
                    AgentId::from_str("ooda-overseer"),
                    None,
                    "task.failed",
                    serde_json::json!({
                        "task_id": task.id.0,
                        "title": task.title,
                        "status": "failed",
                    }),
                ));
            }
        }

        // Kích hoạt chu trình Phản tư & Tự học hỏi (Self-Improvement Loop)
        self.reflect_and_crystallize_lessons(&task, had_fatal_failure, total_remediations_spawned)
            .await;

        Ok(task.status.clone())
    }

    /// Đúc kết bài học có cấu trúc sau khi hoàn thành hoặc thất bại Goal (Self-Improvement Loop)
    async fn reflect_and_crystallize_lessons(
        &self,
        task: &Task,
        had_fatal_failure: bool,
        remediations_count: usize,
    ) {
        info!("🔮 [OODA Self-Improvement] Khởi chạy chu trình phản tư đúc kết kinh nghiệm...");

        let mut errors_encountered = Vec::new();
        let mut remediations_applied = Vec::new();

        for st in &task.subtasks {
            if let Some(ref err) = st.remediation_for_error {
                remediations_applied.push(format!("{}: {}", st.title, st.command_or_prompt));
                errors_encountered.push(err.clone());
            }
            if let TaskStatus::Failed { ref error } = st.status {
                errors_encountered.push(error.clone());
            }
        }

        let trigger_err = if !errors_encountered.is_empty() {
            Some(errors_encountered.join(" | "))
        } else {
            None
        };

        let remediation_str = if !remediations_applied.is_empty() {
            Some(remediations_applied.join("; "))
        } else {
            None
        };

        let general_insight = if had_fatal_failure {
            format!(
                "Goal failed after {} subtasks. Blocker: {}",
                task.subtasks.len(),
                trigger_err.as_deref().unwrap_or("Unknown fatal error")
            )
        } else if remediations_count > 0 {
            format!(
                "Successfully recovered via {} self-corrections. Effective solutions: {}",
                remediations_count,
                remediation_str.as_deref().unwrap_or("Dynamic fix")
            )
        } else {
            format!(
                "Direct path execution succeeded with {} sequential subtasks with zero deviation.",
                task.subtasks.len()
            )
        };

        let lesson = agent_memory::StructuredLesson {
            id: format!("lesson_{}", chrono::Utc::now().timestamp_millis()),
            timestamp: chrono::Utc::now().to_rfc3339(),
            task_title: task.title.clone(),
            trigger_error: trigger_err,
            remediation_applied: remediation_str,
            general_insight,
            success: !had_fatal_failure,
        };

        if let Err(e) = self.durable.save_structured_lesson(&lesson).await {
            warn!(
                "⚠️ [OODA Self-Improvement] Không lưu được bài học có cấu trúc: {}",
                e
            );
        } else {
            info!("🧠 [OODA Self-Improvement] Đã kết tinh thành công bài học [{}] vào bộ nhớ vĩnh cửu.", lesson.id);
        }
    }

    async fn capture_git_diff(workdir: &Path) -> Option<String> {
        let out = tokio::process::Command::new("git")
            .current_dir(workdir)
            .args(["diff", "--stat"])
            .output()
            .await
            .ok()?;
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
        None
    }
}
