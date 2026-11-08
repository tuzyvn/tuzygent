use super::{KernelOperator, SeatStatusReport};
use agent_engine::tmux::TmuxSessionManager;
use tracing::info;

impl KernelOperator {
    /// Khởi động toàn bộ team: tạo các persistent seats (tmux hoặc process)
    pub fn boot_seats(&self) -> Result<Vec<String>, String> {
        self.boot_seats_with(
            TmuxSessionManager::is_available,
            |rig_id, seat_id, workspace_dir| {
                TmuxSessionManager::create_seat_session(rig_id, seat_id, workspace_dir)
            },
        )
    }

    pub(super) fn boot_seats_with<A, C>(
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
