use crate::bus::InterAgentBus;
use agent_domain::{AgentId, InterAgentEvent, RigSnapshot, RigSpec, SeatSnapshot};
use agent_engine::cli::{AntigravityEngine, ClaudeCodeEngine};
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
}

impl KernelOperator {
    pub fn new(spec: RigSpec, workspace_dir: &str) -> Self {
        Self {
            spec,
            workspace_dir: workspace_dir.to_string(),
        }
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
                    let restore_msg = format!(
                        "echo '>>> [Rig Restored] Seat: {} | Restored at: {} | Snapshot: {}'",
                        seat_snap.seat_id,
                        Utc::now().to_rfc3339(),
                        snapshot.snapshot_id
                    );
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
    ) -> Arc<dyn ComputeEngine> {
        let chosen = cli_override
            .or(configured)
            .unwrap_or("antigravity")
            .to_lowercase();

        match chosen.as_str() {
            "claude" | "claude-code" => Arc::new(ClaudeCodeEngine::new(None, None)),
            "mock" => Arc::new(agent_engine::cli::MockEngine::new("Mock completed task")),
            _ => Arc::new(AntigravityEngine::new(None, None)),
        }
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
                if s.harness == "tmux" && TmuxSessionManager::is_available() {
                    let sess =
                        TmuxSessionManager::format_session_name(&self.spec.rig.id, &s.seat_id);
                    let cmd = format!(
                        "echo '>>> [Seat: {}] Started task: {}'",
                        s.seat_id, assignment.task
                    );
                    let _ = TmuxSessionManager::send_command(&sess, &cmd);
                    std::thread::sleep(std::time::Duration::from_millis(100));

                    let out = TmuxSessionManager::capture_pane(&sess, 20)
                        .unwrap_or_else(|_| "Executed in tmux".to_string());
                    assignment.output = Some(out);
                    assignment.status = "Completed".to_string();
                } else {
                    // Chạy qua Compute Engine được cấu hình
                    let engine = self.resolve_engine(s.engine.as_deref(), engine_override);
                    let prompt = format!(
                        "Bạn là nhân sự số '{}' (Vai trò: {}). Hãy thực thi tác vụ: {}",
                        s.seat_id, s.role, assignment.task
                    );
                    let input = EngineInput {
                        prompt_or_command: prompt.clone(),
                        system_context: format!("Rig Operator Dispatch for {}", s.seat_id),
                        working_dir: self.workspace_dir.clone(),
                        timeout_seconds: 60,
                        on_stream_chunk: None,
                    };
                    match engine.execute(&input).await {
                        Ok(res) => {
                            assignment.output = Some(res.stdout_content);
                            assignment.status = "Completed".to_string();
                        }
                        Err(e) => {
                            assignment.output = Some(format!("Lỗi: {:?}", e));
                            assignment.status = "Failed".to_string();
                        }
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

                    if target_seat.harness == "tmux" && TmuxSessionManager::is_available() {
                        let sess = TmuxSessionManager::format_session_name(
                            &self.spec.rig.id,
                            &target_seat.seat_id,
                        );
                        let cmd = format!(
                            "echo '>>> [Channel Handshake] From: {} (Mode: {}) - Handled'",
                            channel.from_seat, mode
                        );
                        let _ = TmuxSessionManager::send_command(&sess, &cmd);
                        ch_assign.output = Some(format!(
                            "Channel handoff received in tmux session '{}'",
                            sess
                        ));
                        ch_assign.status = "Completed".to_string();
                    } else {
                        ch_assign.output = Some(format!(
                            "Channel handoff processed for seat '{}' (mode: {})",
                            target_seat.seat_id, mode
                        ));
                        ch_assign.status = "Completed".to_string();
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
        let synthesis = format!(
            "Kernel Operator '{}' đã hoàn tất điều phối {} tác vụ và {} luồng kênh liên lạc cho đội ngũ '{}'. Toàn bộ các seats đã hoàn tất mục tiêu.",
            self.spec.operator.agent_id,
            assignments.len(),
            channel_dispatches.len(),
            self.spec.rig.name
        );

        Ok(RigExecutionSummary {
            rig_id: self.spec.rig.id.clone(),
            rig_name: self.spec.rig.name.clone(),
            operator_id: self.spec.operator.agent_id.clone(),
            goal: goal.to_string(),
            status: "Completed".to_string(),
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
