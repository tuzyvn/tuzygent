use super::types::{RigExecutionSummary, RigSeatAssignment};
use super::KernelOperator;
use crate::bus::InterAgentBus;
use agent_domain::{AgentId, InterAgentEvent};
use agent_engine::{ComputeEngine, EngineInput};
use chrono::Utc;
use std::path::Path;
use std::sync::Arc;
use tracing::info;

impl KernelOperator {
    /// Giải quyết ComputeEngine theo thứ tự ưu tiên (Configurable Engine):
    /// 1. engine_override từ CLI
    /// 2. engine trong cấu hình YAML
    /// 3. Fallback
    pub fn resolve_engine(
        &self,
        configured: Option<&str>,
        cli_override: Option<&str>,
    ) -> Result<Arc<dyn ComputeEngine>, String> {
        self.resolve_engine_with_model(configured, cli_override, None)
    }

    pub(crate) fn resolve_engine_with_model(
        &self,
        configured: Option<&str>,
        cli_override: Option<&str>,
        model: Option<&str>,
    ) -> Result<Arc<dyn ComputeEngine>, String> {
        let config =
            agent_engine::EnginesConfig::load_hierarchical(Some(Path::new(&self.workspace_dir)));
        let chosen = cli_override
            .or(configured)
            .unwrap_or(&config.defaults.engine)
            .to_lowercase();
        if let Some(engine) = self.engines.get(&chosen) {
            return Ok(engine.clone());
        }
        agent_engine::configured_engine(&config, &chosen, model).map_err(|error| error.to_string())
    }

    /// Kernel Operator tiếp nhận mục tiêu từ người dùng, lập kế hoạch và điều phối các seats
    pub async fn execute_goal(
        &self,
        goal: &str,
        engine_override: Option<&str>,
    ) -> Result<RigExecutionSummary, String> {
        let started_at = Utc::now();
        let execution = crate::execution::ExecutionService::from_workspace(&self.workspace_dir)
            .map_err(|e| e.to_string())?;
        info!(
            "🎯 [KernelOperator] Received goal for Rig '{}': {}",
            self.spec.rig.name, goal
        );

        // Khởi tạo Bus giao tiếp liên agent
        let bus = InterAgentBus::new(128);

        // Đảm bảo các seats đã sẵn sàng
        let _ = self.boot_seats();

        // 1. Operator lập danh sách phân bổ tác vụ dựa theo capabilities của từng seat
        let mut assignments = Vec::new();
        let goal_lower = goal.to_lowercase();

        for seat in &self.spec.seats {
            let matched = seat
                .capabilities
                .iter()
                .any(|c| goal_lower.contains(&c.to_lowercase()));
            let is_assigned = matched || self.spec.seats.len() == 1;

            if is_assigned {
                assignments.push(RigSeatAssignment {
                    seat_id: seat.seat_id.clone(),
                    task: format!(
                        "Thực thi nhiệm vụ '{}' phù hợp năng lực: {:?}",
                        goal, seat.capabilities
                    ),
                    status: "Pending".to_string(),
                    output: None,
                });
            }
        }

        // Nếu không khớp từ khóa nào, phân bổ cho tất cả các seat theo thứ tự
        if assignments.is_empty() {
            for seat in &self.spec.seats {
                assignments.push(RigSeatAssignment {
                    seat_id: seat.seat_id.clone(),
                    task: format!("Thực hiện phần việc cho mục tiêu: {}", goal),
                    status: "Pending".to_string(),
                    output: None,
                });
            }
        }

        // 2. Thực thi phân bổ qua Persistent Seats hoặc ComputeEngine
        for assignment in &mut assignments {
            assignment.status = "Executing".to_string();
            let seat = self.spec.find_seat(&assignment.seat_id);

            // Gửi sự kiện phân bổ lên Bus
            let assign_evt = InterAgentEvent::new(
                AgentId(self.spec.operator.agent_id.clone()),
                Some(AgentId(assignment.seat_id.clone())),
                "rig.task.assigned",
                serde_json::json!({
                    "seat_id": assignment.seat_id,
                    "task": assignment.task,
                }),
            );
            let _ = bus.publish(assign_evt);

            if let Some(s) = seat {
                // Tmux manages seat visibility; task completion comes from a bounded engine run.
                let engine = match self.resolve_engine_with_model(
                    s.engine.as_deref(),
                    engine_override,
                    s.model.as_deref(),
                ) {
                    Ok(eng) => eng,
                    Err(error) => {
                        assignment.status = "Failed".into();
                        assignment.output = Some(format!("Failed to resolve engine: {}", error));
                        let fail_evt = InterAgentEvent::new(
                            AgentId(assignment.seat_id.clone()),
                            Some(AgentId(self.spec.operator.agent_id.clone())),
                            "rig.task.failed",
                            serde_json::json!({
                                "seat_id": assignment.seat_id,
                                "status": assignment.status,
                                "error": assignment.output,
                            }),
                        );
                        let _ = bus.publish(fail_evt);
                        continue;
                    }
                };
                let input = EngineInput {
                    prompt_or_command: format!(
                        "Bạn là '{}' (Vai trò: {}). Thực thi: {}",
                        s.seat_id, s.role, assignment.task
                    ),
                    system_context: format!("Rig Operator Dispatch for {}", s.seat_id),
                    working_dir: self.workspace_dir.clone(),
                    timeout_seconds: 60,
                    on_stream_chunk: None,
                    command_sandbox: None,
                    on_event: None,
                    temperature: None,
                    max_tokens: None,
                    max_output_bytes: None,
                };
                match execution.execute(engine, &input).await {
                    Ok(result) => {
                        assignment.status = if result.exit_code == 0 {
                            "Completed"
                        } else {
                            "Failed"
                        }
                        .into();
                        assignment.output = Some(result.stdout_content);
                    }
                    Err(error) => {
                        assignment.status = "Failed".into();
                        assignment.output = Some(error.to_string());
                    }
                }

                // Gửi sự kiện hoàn thành lên Bus
                let done_evt = InterAgentEvent::new(
                    AgentId(assignment.seat_id.clone()),
                    Some(AgentId(self.spec.operator.agent_id.clone())),
                    "rig.task.completed",
                    serde_json::json!({
                        "seat_id": assignment.seat_id,
                        "status": assignment.status,
                        "output": assignment.output,
                    }),
                );
                let _ = bus.publish(done_evt);
            }
        }

        // 3. Định tuyến kênh liên lạc nội bộ (Rig Channels Pipeline - OpenRig Chaining)
        let mut channel_dispatches = Vec::new();
        for channel in &self.spec.channels {
            if let Some(src_assignment) = assignments
                .iter()
                .find(|a| a.seat_id == channel.from_seat && a.status == "Completed")
            {
                if let Some(target_seat) = self.spec.find_seat(&channel.to_seat) {
                    let mode = channel.mode.as_deref().unwrap_or("direct");
                    let channel_task = format!(
                        "Tiếp nhận kết quả từ {} (Kênh: {}): Kiểm tra và tiếp nối nhiệm vụ",
                        channel.from_seat, mode
                    );

                    info!(
                        "🔀 [Rig Channel] Forwarding output from '{}' to '{}' (Mode: {})",
                        channel.from_seat, channel.to_seat, mode
                    );

                    let mut ch_assign = RigSeatAssignment {
                        seat_id: channel.to_seat.clone(),
                        task: channel_task.clone(),
                        status: "Executing".to_string(),
                        output: None,
                    };

                    let engine = match self.resolve_engine_with_model(
                        target_seat.engine.as_deref(),
                        engine_override,
                        target_seat.model.as_deref(),
                    ) {
                        Ok(eng) => eng,
                        Err(error) => {
                            ch_assign.status = "Failed".into();
                            ch_assign.output =
                                Some(format!("Failed to resolve channel engine: {}", error));
                            let fail_evt = InterAgentEvent::new(
                                AgentId(ch_assign.seat_id.clone()),
                                Some(AgentId(self.spec.operator.agent_id.clone())),
                                "rig.channel.failed",
                                serde_json::json!({
                                    "seat_id": ch_assign.seat_id,
                                    "status": ch_assign.status,
                                    "error": ch_assign.output,
                                }),
                            );
                            let _ = bus.publish(fail_evt);
                            channel_dispatches.push(ch_assign);
                            continue;
                        }
                    };
                    let input = EngineInput {
                        prompt_or_command: channel_task,
                        system_context: format!(
                            "Role: {}\nSource task: {}\nSource output:\n{}",
                            target_seat.role,
                            src_assignment.task,
                            src_assignment.output.as_deref().unwrap_or("")
                        ),
                        working_dir: self.workspace_dir.clone(),
                        timeout_seconds: 60,
                        on_stream_chunk: None,
                        command_sandbox: None,
                        on_event: None,
                        temperature: None,
                        max_tokens: None,
                        max_output_bytes: None,
                    };
                    match execution.execute(engine, &input).await {
                        Ok(result) => {
                            ch_assign.status = if result.exit_code == 0 {
                                "Completed"
                            } else {
                                "Failed"
                            }
                            .into();
                            ch_assign.output = Some(result.stdout_content);
                        }
                        Err(error) => {
                            ch_assign.status = "Failed".into();
                            ch_assign.output = Some(error.to_string());
                        }
                    }

                    // Gửi sự kiện channel lên Bus
                    let ch_evt = InterAgentEvent::new(
                        AgentId(channel.from_seat.clone()),
                        Some(AgentId(channel.to_seat.clone())),
                        &format!("rig.channel.{}", mode),
                        serde_json::json!({
                            "from_seat": channel.from_seat,
                            "to_seat": channel.to_seat,
                            "mode": mode,
                            "src_task": src_assignment.task,
                        }),
                    );
                    let _ = bus.publish(ch_evt);

                    channel_dispatches.push(ch_assign);
                }
            }
        }

        let completed_at = Utc::now();
        let succeeded = !assignments.is_empty()
            && assignments
                .iter()
                .chain(channel_dispatches.iter())
                .all(|assignment| assignment.status == "Completed");
        let synthesis = format!(
            "Rig '{}' processed {} tasks and {} channel handoffs. Status: {}.",
            self.spec.rig.name,
            assignments.len(),
            channel_dispatches.len(),
            if succeeded { "Completed" } else { "Failed" }
        );

        Ok(RigExecutionSummary {
            rig_id: self.spec.rig.id.clone(),
            rig_name: self.spec.rig.name.clone(),
            operator_id: self.spec.operator.agent_id.clone(),
            goal: goal.to_string(),
            status: if succeeded { "Completed" } else { "Failed" }.into(),
            started_at: started_at.to_rfc3339(),
            completed_at: completed_at.to_rfc3339(),
            assignments,
            channel_dispatches,
            operator_synthesis: synthesis,
        })
    }
}
