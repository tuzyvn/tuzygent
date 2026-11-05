//! Process execution and lifecycle management with process group supervision.

use super::{
    builder::{command, timeout},
    stream::{read_stream, DEFAULT_MAX_OUTPUT_BYTES},
};
use crate::{EngineError, EngineInput, StreamCallback};
use std::{collections::HashMap, process::Stdio, time::Duration};
use tokio::process::Command;

#[cfg(unix)]
pub struct ProcessGroup(Option<u32>);

#[cfg(unix)]
impl ProcessGroup {
    pub fn new(pid: Option<u32>) -> Self {
        Self(pid)
    }

    pub fn is_armed(&self) -> bool {
        self.0.is_some()
    }

    pub fn pid(&self) -> Option<u32> {
        self.0
    }

    pub fn disarm(&mut self) -> Option<u32> {
        self.0.take()
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            // Guard against invalid, init (pid 1), or sign-overflow PIDs (pid > i32::MAX).
            // Pid::from_raw(-1) in killpg would broadcast SIGKILL to ALL processes owned by caller.
            if pid > 1 && pid <= (i32::MAX as u32) {
                use nix::{
                    sys::signal::{killpg, Signal},
                    unistd::Pid,
                };
                let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
            }
        }
    }
}

/// Drains both pipes concurrently; kills the process group on timeout or cancellation.
pub async fn run_process(
    cmd: Command,
    timeout: Duration,
    callback: Option<StreamCallback>,
) -> Result<std::process::Output, EngineError> {
    run_process_with_limit(cmd, timeout, callback, DEFAULT_MAX_OUTPUT_BYTES).await
}

pub async fn run_process_with_limit(
    mut cmd: Command,
    timeout: Duration,
    callback: Option<StreamCallback>,
    max_output_bytes: usize,
) -> Result<std::process::Output, EngineError> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| EngineError::ProcessFailed(e.to_string()))?;
    #[cfg(unix)]
    let group = ProcessGroup(child.id());
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| EngineError::ProcessFailed("Failed to capture stdout pipe".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| EngineError::ProcessFailed("Failed to capture stderr pipe".into()))?;

    let effective_timeout = if timeout.is_zero() {
        Duration::from_secs(300)
    } else {
        timeout
    };

    let effective_limit = if max_output_bytes == 0 {
        DEFAULT_MAX_OUTPUT_BYTES
    } else {
        max_output_bytes
    };

    let run = async {
        let (status, stdout, stderr) = tokio::try_join!(
            child.wait(),
            read_stream(stdout, callback, effective_limit),
            read_stream(stderr, None, effective_limit)
        )?;
        Ok::<_, std::io::Error>(std::process::Output {
            status,
            stdout,
            stderr,
        })
    };
    match tokio::time::timeout(effective_timeout, run).await {
        Ok(result) => result.map_err(|e| EngineError::ProcessFailed(e.to_string())),
        Err(_) => {
            #[cfg(unix)]
            drop(group);
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(EngineError::Timeout)
        }
    }
}

struct ContainerCleanup(Option<String>);
impl Drop for ContainerCleanup {
    fn drop(&mut self) {
        if let Some(name) = self.0.take() {
            // Drop also runs when an outer future is cancelled. Docker owns the container
            // independently of the CLI process, so killing that process is insufficient.
            std::thread::spawn(move || {
                let _ = std::process::Command::new("docker")
                    .args(["rm", "--force", &name])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            });
        }
    }
}

pub async fn run_command(
    input: &EngineInput,
    binary: &str,
    args: &[String],
    env: &HashMap<String, String>,
    callback: Option<StreamCallback>,
) -> Result<std::process::Output, EngineError> {
    if binary.trim().is_empty() {
        return Err(EngineError::ProcessFailed(
            "Command binary cannot be empty".to_string(),
        ));
    }
    let cleanup = ContainerCleanup(
        input
            .command_sandbox
            .as_ref()
            .and_then(|sandbox| sandbox.container_name.clone()),
    );
    let limit = match input.max_output_bytes {
        Some(0) | None => DEFAULT_MAX_OUTPUT_BYTES,
        Some(bytes) => bytes,
    };
    let result = run_process_with_limit(
        command(input, binary, args, env),
        timeout(input),
        callback,
        limit,
    )
    .await;
    if let Some(name) = &cleanup.0 {
        // Wait for cleanup on normal completion and timeout; Drop handles cancellation.
        let mut remove = Command::new("docker");
        remove.args(["rm", "--force", name]);
        let _ = run_process(remove, Duration::from_secs(10), None).await;
    }
    // Drop retries cleanup if cancellation interrupted the awaited removal.
    let mut cleanup = cleanup;
    cleanup.0 = None;
    result
}

/// Runs a simple command with default settings and 300s timeout.
pub async fn run_simple_command(
    binary: &str,
    args: &[&str],
) -> Result<std::process::Output, EngineError> {
    let input = EngineInput::new("", ".");
    let string_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let env = HashMap::new();
    run_command(&input, binary, &string_args, &env, None).await
}
