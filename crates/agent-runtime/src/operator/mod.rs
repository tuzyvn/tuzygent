mod execution;
mod snapshot;
mod types;

pub use types::{RigExecutionSummary, RigSeatAssignment, SeatStatusReport};

use agent_domain::RigSpec;
use agent_engine::tmux::TmuxSessionManager;
use agent_engine::ComputeEngine;
use std::sync::Arc;
use tracing::{info, warn};

pub struct KernelOperator {
    pub spec: RigSpec,
    pub workspace_dir: String,
    pub(crate) engines: std::collections::HashMap<String, Arc<dyn ComputeEngine>>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_domain::{RigChannelConfig, RigMetadata, RigOperatorConfig, RigSeatConfig};

    #[test]
    fn test_boot_and_shutdown_process_seats() {
        let spec = RigSpec {
            rig: RigMetadata {
                id: "seat-lifecycle-test".to_string(),
                name: "Seat lifecycle test".to_string(),
                description: None,
                version: None,
            },
            operator: RigOperatorConfig {
                agent_id: "operator".to_string(),
                name: None,
                role: None,
                persona_path: None,
                engine: None,
                model: None,
            },
            seats: vec![
                RigSeatConfig {
                    seat_id: "worker-1".to_string(),
                    name: None,
                    role: "worker".to_string(),
                    capabilities: vec![],
                    persona_path: None,
                    engine: None,
                    model: None,
                    harness: "process".to_string(),
                    voice_preset: None,
                    autonomy: "full".to_string(),
                },
                RigSeatConfig {
                    seat_id: "worker-2".to_string(),
                    name: None,
                    role: "worker".to_string(),
                    capabilities: vec![],
                    persona_path: None,
                    engine: None,
                    model: None,
                    harness: "process".to_string(),
                    voice_preset: None,
                    autonomy: "full".to_string(),
                },
            ],
            channels: vec![],
        };

        let operator = KernelOperator::new(spec, ".");

        assert_eq!(
            operator.boot_seats().unwrap(),
            vec!["process_worker-1", "process_worker-2"]
        );
        assert!(operator.shutdown_seats().is_ok());
    }

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
