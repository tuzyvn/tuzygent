use std::time::Duration;
use tokio::process::Command;
use tracing::warn;

#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

/// Bộ điều khiển Process Tree theo quy chuẩn Multica:
/// Khi một tiến trình hoặc công cụ của Agent bị cancel / timeout,
/// toàn bộ nhóm tiến trình con (PGID) phải bị triệt tiêu hoàn toàn,
/// không để lại orphan/zombie process giữ file lock hoặc repo lock.
pub struct ProcessTreeController;

impl ProcessTreeController {
    /// Cấu hình Command chạy trong một Process Group riêng biệt (Setpgid)
    #[cfg(unix)]
    pub fn configure_command(cmd: &mut Command) {
        unsafe {
            cmd.pre_exec(|| {
                // Tạo Process Group mới với PGID = PID của tiến trình
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(not(unix))]
    pub fn configure_command(_cmd: &mut Command) {}

    /// Gửi SIGTERM tới toàn bộ Process Group để dừng nhẹ nhàng
    #[cfg(unix)]
    pub fn interrupt_group(pid: u32) {
        let pgid = Pid::from_raw(pid as i32);
        // Gửi signal tới -PGID (âm) để tác động toàn bộ nhóm
        let _ = kill(Pid::from_raw(-(pgid.as_raw())), Signal::SIGTERM);
    }

    /// Gửi SIGKILL tới toàn bộ Process Group để ép buộc dừng
    #[cfg(unix)]
    pub fn kill_group(pid: u32) {
        let pgid = Pid::from_raw(pid as i32);
        let _ = kill(Pid::from_raw(-(pgid.as_raw())), Signal::SIGKILL);
    }

    #[cfg(not(unix))]
    pub fn interrupt_group(_pid: u32) {}
    #[cfg(not(unix))]
    pub fn kill_group(_pid: u32) {}

    /// Chạy Command với cơ chế bảo vệ Process Tree và Timeout
    pub async fn run_bounded(
        mut cmd: Command,
        timeout: Duration,
        graceful_stop_timeout: Duration,
    ) -> Result<std::process::Output, String> {
        Self::configure_command(&mut cmd);

        let child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn process: {}", e))?;

        let pid = child.id();

        let wait_fut = child.wait_with_output();

        match tokio::time::timeout(timeout, wait_fut).await {
            Ok(output_res) => output_res.map_err(|e| format!("Process execution failed: {}", e)),
            Err(_) => {
                // Hết hạn timeout: Tiến hành quy trình ngắt 2 pha chuẩn Multica
                warn!(
                    "⏳ [ProcessTree] Command timed out after {:?}. Initiating graceful kill...",
                    timeout
                );
                if let Some(p) = pid {
                    Self::interrupt_group(p);

                    // Chờ graceful shutdown
                    tokio::time::sleep(graceful_stop_timeout).await;

                    // Nếu vẫn còn sống, ép dừng bằng SIGKILL
                    warn!(
                        "💀 [ProcessTree] Graceful timeout elapsed for PID {}. Forcing SIGKILL...",
                        p
                    );
                    Self::kill_group(p);
                }

                Err(format!("Process tree timed out after {:?}", timeout))
            }
        }
    }
}
