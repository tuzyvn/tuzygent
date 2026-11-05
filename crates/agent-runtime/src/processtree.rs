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
    /// Cấu hình Command chạy trong một Process Group riêng biệt (Setpgid).
    ///
    /// # Safety Invariants
    /// - Hàm `pre_exec` chạy trong ngữ cảnh child process sau `fork()` và trước `exec()`.
    /// - Trong ngữ cảnh này, việc cấp phát bộ nhớ heap (malloc), lấy khoá (mutex), hoặc gọi các hàm
    ///   không thuộc danh mục POSIX async-signal-safe có thể dẫn tới deadlock hoặc undefined behavior.
    /// - Lệnh gọi `libc::setpgid(0, 0)` là một POSIX system call hoàn toàn async-signal-safe, không
    ///   cấp phát bộ nhớ và không tranh chấp tài nguyên, an toàn để thiết lập Process Group mới
    ///   (với PGID bằng PID của child process).
    #[cfg(unix)]
    pub fn configure_command(cmd: &mut Command) {
        cmd.kill_on_drop(true);
        // SAFETY: `pre_exec` chỉ thực hiện `libc::setpgid(0, 0)`, một hàm chuẩn POSIX async-signal-safe,
        // không cấp phát heap hay chiếm giữ lock, đảm bảo an toàn tuyệt đối sau `fork()`.
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
    pub fn configure_command(cmd: &mut Command) {
        cmd.kill_on_drop(true);
    }

    /// Gửi SIGTERM tới toàn bộ Process Group để dừng nhẹ nhàng.
    /// Ranh giới: Bỏ qua an toàn nếu PID = 0 (tránh gửi tín hiệu tới process group của chính caller)
    /// hoặc vượt quá giới hạn i32::MAX.
    #[cfg(unix)]
    pub fn interrupt_group(pid: u32) {
        if pid == 0 || pid > i32::MAX as u32 {
            return;
        }
        let pgid = pid as i32;
        // Gửi signal tới -PGID (âm) để tác động toàn bộ nhóm
        let _ = kill(Pid::from_raw(-pgid), Signal::SIGTERM);
    }

    /// Gửi SIGKILL tới toàn bộ Process Group để ép buộc dừng.
    /// Ranh giới: Bỏ qua an toàn nếu PID = 0 (tránh gửi tín hiệu tới process group của chính caller)
    /// hoặc vượt quá giới hạn i32::MAX.
    #[cfg(unix)]
    pub fn kill_group(pid: u32) {
        if pid == 0 || pid > i32::MAX as u32 {
            return;
        }
        let pgid = pid as i32;
        let _ = kill(Pid::from_raw(-pgid), Signal::SIGKILL);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn test_configure_command_sets_pgid_equal_to_pid() {
        use nix::unistd::getpgid;

        let mut cmd = Command::new("sleep");
        cmd.arg("10");
        cmd.kill_on_drop(true);
        ProcessTreeController::configure_command(&mut cmd);

        let mut child = cmd.spawn().expect("failed to spawn child process");
        let pid = child.id().expect("child should have a pid");

        let pgid = getpgid(Some(Pid::from_raw(pid as i32)))
            .expect("failed to get pgid of child process");

        // Verify PGID equals the child PID
        assert_eq!(
            pgid.as_raw(),
            pid as i32,
            "Spawned process PGID must equal its own PID"
        );

        // Clean up spawned process
        let _ = child.kill().await;
        let _ = child.wait().await;
    }

    #[test]
    fn test_process_tree_boundary_pid_zero_and_overflow_does_not_signal_caller() {
        // Nếu pid = 0 không được chặn, kill(Pid::from_raw(0), ...) sẽ gửi signal tới caller process group
        // và làm crash tiến trình hiện tại. Test này xác nhận việc gọi an toàn tuyệt đối.
        ProcessTreeController::interrupt_group(0);
        ProcessTreeController::kill_group(0);

        // Kiểm tra với giá trị vượt ngưỡng i32::MAX
        ProcessTreeController::interrupt_group(u32::MAX);
        ProcessTreeController::kill_group(u32::MAX);
        ProcessTreeController::interrupt_group(i32::MAX as u32 + 1);
        ProcessTreeController::kill_group(i32::MAX as u32 + 1);
    }

    #[tokio::test]
    async fn test_run_bounded_success() {
        let mut cmd = Command::new("echo");
        cmd.arg("multica_processtree_ok");
        cmd.stdout(std::process::Stdio::piped());

        let output = ProcessTreeController::run_bounded(
            cmd,
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .await
        .expect("run_bounded should succeed for echo command");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("multica_processtree_ok"));
    }

    #[tokio::test]
    async fn test_run_bounded_nonexistent_command_returns_err() {
        let cmd = Command::new("__nonexistent_command_binary_12345__");

        let err = ProcessTreeController::run_bounded(
            cmd,
            Duration::from_secs(2),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();

        assert!(err.contains("Failed to spawn process"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_run_bounded_timeout_terminates_gracefully() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");

        let start = std::time::Instant::now();
        let err = ProcessTreeController::run_bounded(
            cmd,
            Duration::from_millis(100),
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("timed out"),
            "Error should mention timeout: {}",
            err
        );
        // Đảm bảo không bị block quá lâu (phải hoàn tất sau khoảng timeout + graceful)
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}

