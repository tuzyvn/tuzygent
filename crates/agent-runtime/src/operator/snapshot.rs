use super::KernelOperator;
use agent_domain::{RigSnapshot, SeatSnapshot};
use agent_engine::tmux::TmuxSessionManager;
use chrono::Utc;
use std::path::Path;
use tracing::info;

impl KernelOperator {
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
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("Failed to create snapshot directory {:?}: {}", dir, e))?;
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
}
