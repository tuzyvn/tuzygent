mod execution;
mod snapshot;
mod types;

pub use types::{RigExecutionSummary, RigSeatAssignment, SeatStatusReport};

use agent_domain::RigSpec;
use agent_engine::tmux::TmuxSessionManager;
use agent_engine::ComputeEngine;
use std::sync::Arc;
use tracing::info;

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
        self.boot_seats_with(
            TmuxSessionManager::is_available,
            |rig_id, seat_id, workspace_dir| {
                TmuxSessionManager::create_seat_session(rig_id, seat_id, workspace_dir)
            },
        )
    }

    fn boot_seats_with<A, C>(
        &self,
        tmux_is_available: A,
        mut create_tmux_session: C,
    ) -> Result<Vec<String>, String>
    where
        A: Fn() -> bool,
        C: FnMut(&str, &str, Option<&str>) -> Result<String, String>,
    {
        let mut booted = Vec::new();
        for seat in &self.spec.seats {
            if seat.harness == "tmux" && tmux_is_available() {
                match create_tmux_session(
                    &self.spec.rig.id,
                    &seat.seat_id,
                    Some(&self.workspace_dir),
                ) {
                    Ok(sess) => {
                        info!("🚀 Booted tmux seat '{}' -> '{}'", seat.seat_id, sess);
                        booted.push(sess);
                    }
                    Err(e) => {
                        return Err(format!(
                            "Failed to boot tmux seat '{}': {}",
                            seat.seat_id, e
                        ));
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

    struct TestSnapshotFile(std::path::PathBuf);

    impl TestSnapshotFile {
        fn new(label: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "agent-runtime-snapshot-{label}-{}.json",
                uuid::Uuid::new_v4()
            )))
        }

        fn as_str(&self) -> &str {
            self.0.to_str().expect("temporary path must be valid UTF-8")
        }
    }

    impl Drop for TestSnapshotFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn snapshot_test_operator() -> KernelOperator {
        KernelOperator::new(
            RigSpec {
                rig: RigMetadata {
                    id: "snapshot-test-rig".to_string(),
                    name: "Snapshot test rig".to_string(),
                    description: None,
                    version: None,
                },
                operator: RigOperatorConfig {
                    agent_id: "snapshot-test-operator".to_string(),
                    name: None,
                    role: None,
                    persona_path: None,
                    engine: None,
                    model: None,
                },
                seats: vec![],
                channels: vec![],
            },
            ".",
        )
    }

    #[test]
    fn test_save_snapshot_to_custom_path_and_load_it_back() {
        let operator = snapshot_test_operator();
        let snapshot = operator.create_snapshot().expect("Must create snapshot");
        let snapshot_file = TestSnapshotFile::new("roundtrip");

        let saved_path = operator
            .save_snapshot(&snapshot, Some(snapshot_file.as_str()))
            .expect("Must save snapshot to custom path");

        assert_eq!(saved_path, snapshot_file.as_str());
        assert!(snapshot_file.0.is_file());
        assert_eq!(
            KernelOperator::load_snapshot(snapshot_file.as_str())
                .expect("Must load saved snapshot"),
            snapshot
        );
    }

    #[test]
    fn test_load_snapshot_returns_error_for_missing_or_invalid_json_file() {
        let missing_file = TestSnapshotFile::new("missing");
        assert!(!missing_file.0.exists());
        assert!(KernelOperator::load_snapshot(missing_file.as_str()).is_err());

        let invalid_file = TestSnapshotFile::new("invalid");
        std::fs::write(&invalid_file.0, "{ this is not valid JSON")
            .expect("Must write invalid JSON fixture");
        assert!(KernelOperator::load_snapshot(invalid_file.as_str()).is_err());
    }

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
            operator.boot_seats(),
            Ok(vec![
                "process_worker-1".to_string(),
                "process_worker-2".to_string()
            ])
        );
        assert!(operator.shutdown_seats().is_ok());
    }

    #[test]
    fn test_boot_seats_returns_error_when_tmux_creation_fails() {
        let operator = KernelOperator::new(
            RigSpec {
                rig: RigMetadata {
                    id: "tmux-error-test".to_string(),
                    name: "Tmux error test".to_string(),
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
                seats: vec![RigSeatConfig {
                    seat_id: "worker".to_string(),
                    name: None,
                    role: "worker".to_string(),
                    capabilities: vec![],
                    persona_path: None,
                    engine: None,
                    model: None,
                    harness: "tmux".to_string(),
                    voice_preset: None,
                    autonomy: "full".to_string(),
                }],
                channels: vec![],
            },
            ".",
        );

        let result = operator.boot_seats_with(
            || true,
            |_, _, _| Err("simulated tmux startup failure".to_string()),
        );

        assert!(result
            .expect_err("tmux creation failure must be returned, not swallowed")
            .contains("worker"));
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
