use crate::bus::InterAgentBus;
use agent_domain::{AgentId, InterAgentEvent, RigSnapshot, RigSpec, SeatSnapshot};
use agent_engine::tmux::TmuxSessionManager;
use agent_engine::{ComputeEngine, EngineInput};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeatStatusReport {
    pub seat_id: String,
    pub name: String,
    pub role: String,
    pub engine: String,
    pub harness: String,
    pub is_active: bool,
    pub session_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigSeatAssignment {
    pub seat_id: String,
    pub task: String,
    pub status: String,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigExecutionSummary {
    pub rig_id: String,
    pub rig_name: String,
    pub operator_id: String,
    pub goal: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: String,
    pub assignments: Vec<RigSeatAssignment>,
    #[serde(default)]
    pub channel_dispatches: Vec<RigSeatAssignment>,
    pub operator_synthesis: String,
}

pub struct KernelOperator {
    pub spec: RigSpec,
    pub workspace_dir: String,
    engines: std::collections::HashMap<String, Arc<dyn ComputeEngine>>,
}

impl KernelOperator {
    pub fn new(spec: RigSpec, workspace_dir: &str) -> Self {
        Self {
            spec,
            workspace_dir: workspace_dir.to_string(),
            engines: Default::default(),
        }
    }

    /// Register an SDK-provided compute engine for this operator.
    pub fn with_engine(mut self, name: &str, engine: Arc<dyn ComputeEngine>) -> Self {
        self.engines.insert(name.to_lowercase(), engine);
        self
    }

    /// Khởi động toàn bộ team: tạo các persistent seats (tmux hoặc process)
    pub fn boot_seats(&self) -> Result<Vec<String>, String> {
        let mut booted = Vec::new();
        for seat in &self.spec.seats {
            if seat.harness == "tmux" && TmuxSessionManager::is_available() {
                match TmuxSessionManager::create_seat_session(
                    &self.spec.rig.id,
                    &seat.seat_id,
                    Some(&self.workspace_dir),
                ) {
                    Ok(sess) => {
                        info!("🚀 Booted tmux seat '{}' -> '{}'", seat.seat_id, sess);
                        booted.push(sess);
                    }
                    Err(e) => {
                        warn!("Failed to boot tmux seat '{}': {}", seat.seat_id, e);
                    }
                }
            } else {
                info!(
                    "🚀 Seat '{}' configured as in-memory / process harness",
                    seat.seat_id
                );
                booted.push(format!("process_{}", seat.seat_id));
            }
        }
        Ok(booted)
    }

    /// Dừng toàn bộ persistent seats của Rig
    pub fn shutdown_seats(&self) -> Result<(), String> {
        for seat in &self.spec.seats {
            if seat.harness == "tmux" && TmuxSessionManager::is_available() {
                let session_name =
                    TmuxSessionManager::format_session_name(&self.spec.rig.id, &seat.seat_id);
                let _ = TmuxSessionManager::kill_seat_session(&session_name);
            }
        }
        info!(
            "🛑 All seats for rig '{}' have been stopped",
            self.spec.rig.id
        );
        Ok(())
    }

    /// Kiểm tra trạng thái hiện tại của các seats trong Rig
    pub fn inspect_seats(&self) -> Vec<SeatStatusReport> {
        let mut reports = Vec::new();
        for seat in &self.spec.seats {
            let session_name = if seat.harness == "tmux" {
                Some(TmuxSessionManager::format_session_name(
                    &self.spec.rig.id,
                    &seat.seat_id,
                ))
            } else {
                None
            };

            let is_active = match &session_name {
                Some(sess) => TmuxSessionManager::has_session(sess),
                None => true,
            };

            reports.push(SeatStatusReport {
                seat_id: seat.seat_id.clone(),
                name: seat.name.clone().unwrap_or_else(|| seat.seat_id.clone()),
                role: seat.role.clone(),
                engine: seat.engine.clone().unwrap_or_else(|| "default".to_string()),
                harness: seat.harness.clone(),
                is_active,
                session_name,
            });
        }
        reports
    }

    /// Tạo Snapshot lưu lại toàn bộ trạng thái và buffer terminal của các seats
    pub fn create_snapshot(&self) -> Result<RigSnapshot, String> {
        let mut seat_snapshots = Vec::new();
        for seat in &self.spec.seats {
            let (is_active, terminal_buffer) =
                if seat.harness == "tmux" && TmuxSessionManager::is_available() {
                    let sess =
                        TmuxSessionManager::format_session_name(&self.spec.rig.id, &seat.seat_id);
                    let active = TmuxSessionManager::has_session(&sess);
                    let buffer = if active {
                        TmuxSessionManager::capture_pane(&sess, 50).ok()
                    } else {
                        None
                    };
                    (active, buffer)
                } else {
                    (true, None)
                };

            seat_snapshots.push(SeatSnapshot {
                seat_id: seat.seat_id.clone(),
                name: seat.name.clone().unwrap_or_else(|| seat.seat_id.clone()),
                role: seat.role.clone(),
                harness: seat.harness.clone(),
                is_active,
                terminal_buffer,
            });
        }

        let spec_yaml = self.spec.to_yaml_str().map_err(|e| e.to_string())?;
        let snapshot_id = format!(
            "snap_{}_{}",
            self.spec.rig.id,
            Utc::now().format("%Y%m%d_%H%M%S")
        );

        Ok(RigSnapshot {
            snapshot_id,
            rig_id: self.spec.rig.id.clone(),
            rig_name: self.spec.rig.name.clone(),
            created_at: Utc::now().to_rfc3339(),
            seats: seat_snapshots,
            spec_yaml,
        })
    }

    /// Lưu snapshot ra đĩa (mặc định tại storage/rigs/snapshots/)
    pub fn save_snapshot(
        &self,
        snapshot: &RigSnapshot,
        custom_path: Option<&str>,
    ) -> Result<String, String> {
        let target_path = match custom_path {
            Some(p) => p.to_string(),
            None => {
                let dir = Path::new("storage/rigs/snapshots");
                let _ = std::fs::create_dir_all(dir);
                format!("storage/rigs/snapshots/{}.json", snapshot.snapshot_id)
            }
        };

        let json = snapshot
            .to_json()
            .map_err(|e| format!("Serialization error: {}", e))?;
        std::fs::write(&target_path, json)
            .map_err(|e| format!("Failed to write snapshot to {}: {}", target_path, e))?;
        info!("📸 Rig snapshot saved successfully to '{}'", target_path);
        Ok(target_path)
    }

    /// Tải snapshot từ file JSON
    pub fn load_snapshot(file_path: &str) -> Result<RigSnapshot, String> {
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| format!("Failed to read snapshot file '{}': {}", file_path, e))?;
        RigSnapshot::from_json(&content).map_err(|e| format!("Invalid snapshot format: {}", e))
    }

    /// Khôi phục trạng thái Rig từ snapshot
    pub fn restore_snapshot(&self, snapshot: &RigSnapshot) -> Result<(), String> {
        info!(
            "🔄 Restoring Rig '{}' from snapshot '{}'...",
            self.spec.rig.id, snapshot.snapshot_id
        );
        // 1. Boot lại các seats nếu chưa chạy
        self.boot_seats()?;

        // 2. Với các tmux seat, gửi thông điệp thông báo khôi phục
        for seat_snap in &snapshot.seats {
            if seat_snap.harness == "tmux" && TmuxSessionManager::is_available() {
                let sess =
                    TmuxSessionManager::format_session_name(&self.spec.rig.id, &seat_snap.seat_id);
                if TmuxSessionManager::has_session(&sess) {
                    let message = format!(
                        "Rig restored: seat {} | snapshot {}",
                        seat_snap.seat_id, snapshot.snapshot_id
                    );
                    let restore_msg =
                        format!("printf '%s\\n' '{}'", message.replace('\'', "'\\''"));
                    let _ = TmuxSessionManager::send_command(&sess, &restore_msg);
                }
            }
        }
        info!(
            "✅ Rig restored successfully from snapshot '{}'",
            snapshot.snapshot_id
        );
        Ok(())
    }

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

    fn resolve_engine_with_model(
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
                let engine = self.resolve_engine_with_model(
                    s.engine.as_deref(),
                    engine_override,
                    s.model.as_deref(),
                )?;
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
                };
                match engine.execute(&input).await {
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

                    let engine = self.resolve_engine_with_model(
                        target_seat.engine.as_deref(),
                        engine_override,
                        target_seat.model.as_deref(),
                    )?;
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
                    };
                    match engine.execute(&input).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use agent_domain::{RigChannelConfig, RigMetadata, RigOperatorConfig, RigSeatConfig};

    #[tokio::test]
    async fn test_kernel_operator_lifecycle_and_channels() {
        let spec = RigSpec {
            rig: RigMetadata {
                id: "test-rig".to_string(),
                name: "Test Rig".to_string(),
                description: None,
                version: Some("1.0.0".to_string()),
            },
            operator: RigOperatorConfig {
                agent_id: "lead-op".to_string(),
                name: Some("Lead".to_string()),
                role: Some("Operator".to_string()),
                persona_path: None,
                engine: Some("mock".to_string()),
                model: None,
            },
            seats: vec![
                RigSeatConfig {
                    seat_id: "dev-01".to_string(),
                    name: Some("Developer".to_string()),
                    role: "Coder".to_string(),
                    capabilities: vec!["code".to_string(), "rust".to_string()],
                    persona_path: None,
                    engine: Some("mock".to_string()),
                    model: None,
                    harness: "process".to_string(),
                    voice_preset: None,
                    autonomy: "full".to_string(),
                },
                RigSeatConfig {
                    seat_id: "qa-01".to_string(),
                    name: Some("Auditor".to_string()),
                    role: "QA".to_string(),
                    capabilities: vec!["audit".to_string()],
                    persona_path: None,
                    engine: Some("mock".to_string()),
                    model: None,
                    harness: "process".to_string(),
                    voice_preset: None,
                    autonomy: "full".to_string(),
                },
            ],
            channels: vec![RigChannelConfig {
                from_seat: "dev-01".to_string(),
                to_seat: "qa-01".to_string(),
                mode: Some("review".to_string()),
            }],
        };

        let op = KernelOperator::new(spec, ".");
        let status = op.inspect_seats();
        assert_eq!(status.len(), 2);
        assert_eq!(status[0].seat_id, "dev-01");

        // Test Snapshot Lifecycle
        let snapshot = op.create_snapshot().expect("Must create snapshot");
        assert_eq!(snapshot.rig_id, "test-rig");
        assert_eq!(snapshot.seats.len(), 2);

        // Test Restore
        let restore_res = op.restore_snapshot(&snapshot);
        assert!(restore_res.is_ok());

        // Test Goal Execution with Channel Forwarding
        let summary = op
            .execute_goal("Viết code Rust", Some("mock"))
            .await
            .expect("Must execute");
        assert_eq!(summary.status, "Completed");
        assert_eq!(summary.assignments.len(), 1);
        assert_eq!(summary.assignments[0].seat_id, "dev-01");
        // Verify channel dispatched from dev-01 to qa-01
        assert_eq!(summary.channel_dispatches.len(), 1);
        assert_eq!(summary.channel_dispatches[0].seat_id, "qa-01");
    }
}
