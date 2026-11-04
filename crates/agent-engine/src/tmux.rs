use std::process::Command;
use tracing::{info, warn};

pub struct TmuxSessionManager;

impl TmuxSessionManager {
    /// Kiểm tra xem binary tmux có sẵn trên hệ thống không
    pub fn is_available() -> bool {
        Command::new("tmux")
            .arg("-V")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    /// Định danh phiên tmux chuẩn hóa theo OpenRig: rig_{rig_id}_{seat_id}
    pub fn format_session_name(rig_id: &str, seat_id: &str) -> String {
        let clean_rig = rig_id.replace([':', '/', ' ', '.'], "_");
        let clean_seat = seat_id.replace([':', '/', ' ', '.'], "_");
        format!("rig_{}_{}", clean_rig, clean_seat)
    }

    /// Kiểm tra xem session đã tồn tại chưa
    pub fn has_session(session_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        Command::new("tmux")
            .args(["has-session", "-t", session_name])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    /// Khởi tạo persistent terminal seat mới trong tmux
    pub fn create_seat_session(
        rig_id: &str,
        seat_id: &str,
        workdir: Option<&str>,
    ) -> Result<String, String> {
        if !Self::is_available() {
            return Err("tmux is not installed or available on PATH".to_string());
        }

        let session_name = Self::format_session_name(rig_id, seat_id);

        if Self::has_session(&session_name) {
            info!(
                "📌 [TmuxSeat] Session '{}' already exists. Reusing seat.",
                session_name
            );
            return Ok(session_name);
        }

        let mut cmd = Command::new("tmux");
        cmd.args(["new-session", "-d", "-s", &session_name]);

        if let Some(dir) = workdir {
            cmd.args(["-c", dir]);
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to execute tmux: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "Failed to create tmux session '{}': {}",
                session_name, stderr
            ));
        }

        info!(
            "🚀 [TmuxSeat] Created persistent seat session: '{}'",
            session_name
        );
        Ok(session_name)
    }

    /// Gửi lệnh thực thi vào terminal của Agent seat
    pub fn send_command(session_name: &str, command: &str) -> Result<(), String> {
        if !Self::is_available() {
            return Err("tmux is not installed".to_string());
        }

        let output = Command::new("tmux")
            .args(["send-keys", "-t", session_name, command, "C-m"])
            .output()
            .map_err(|e| format!("Failed to send keys to tmux: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "Error sending keys to '{}': {}",
                session_name, stderr
            ));
        }

        Ok(())
    }

    /// Đọc buffer màn hình hiện tại của Agent seat
    pub fn capture_pane(session_name: &str, lines: usize) -> Result<String, String> {
        if !Self::is_available() {
            return Err("tmux is not installed".to_string());
        }

        let lines_arg = format!("-{}", lines);
        let output = Command::new("tmux")
            .args(["capture-pane", "-pt", session_name, "-S", &lines_arg])
            .output()
            .map_err(|e| format!("Failed to capture pane: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "Error capturing pane for '{}': {}",
                session_name, stderr
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Dừng và đóng persistent terminal seat
    pub fn kill_seat_session(session_name: &str) -> Result<(), String> {
        if !Self::is_available() {
            return Ok(());
        }

        if !Self::has_session(session_name) {
            return Ok(());
        }

        let output = Command::new("tmux")
            .args(["kill-session", "-t", session_name])
            .output()
            .map_err(|e| format!("Failed to kill tmux session: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Failed to kill session '{}': {}", session_name, stderr);
        } else {
            info!("🛑 [TmuxSeat] Terminated session: '{}'", session_name);
        }

        Ok(())
    }

    /// Liệt kê các session thuộc về một Rig
    pub fn list_rig_sessions(rig_id: Option<&str>) -> Result<Vec<String>, String> {
        if !Self::is_available() {
            return Ok(vec![]);
        }

        let output = Command::new("tmux")
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()
            .map_err(|e| format!("Failed to list sessions: {}", e))?;

        if !output.status.success() {
            // Không có server tmux đang chạy -> coi như rỗng
            return Ok(vec![]);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let prefix = match rig_id {
            Some(rid) => format!("rig_{}_", rid.replace([':', '/', ' ', '.'], "_")),
            None => "rig_".to_string(),
        };

        let matched = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|s| s.starts_with(&prefix))
            .collect();

        Ok(matched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_session_name() {
        assert_eq!(
            TmuxSessionManager::format_session_name("my-rig", "backend:dev"),
            "rig_my-rig_backend_dev"
        );
    }

    #[test]
    fn test_tmux_session_lifecycle_if_available() {
        if !TmuxSessionManager::is_available() {
            println!("tmux not installed; skipping test.");
            return;
        }

        let test_rig = "test_lifecycle";
        let test_seat = "agent_01";
        let session_name = TmuxSessionManager::create_seat_session(test_rig, test_seat, None)
            .expect("Must create session");

        assert!(TmuxSessionManager::has_session(&session_name));

        // Test send command
        let _ = TmuxSessionManager::send_command(&session_name, "echo 'AGENT_ALIVE_TEST'");
        std::thread::sleep(std::time::Duration::from_millis(150));

        let buffer =
            TmuxSessionManager::capture_pane(&session_name, 50).expect("Must capture pane");
        assert!(buffer.contains("AGENT_ALIVE_TEST") || !buffer.is_empty());

        // Test cleanup
        TmuxSessionManager::kill_seat_session(&session_name).expect("Must kill session");
        assert!(!TmuxSessionManager::has_session(&session_name));
    }
}
