//! Shared command construction and bounded process execution for CLI engines.
use crate::{EngineError, EngineInput, StreamCallback};
use std::{collections::HashMap, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
};

/// Trusted, runtime-created wrapper. Never accepted from serialized engine inputs.
#[derive(Debug, Clone)]
pub struct CommandSandbox {
    pub program: String,
    pub prefix_args: Vec<String>,
    pub env_whitelist: Vec<String>,
    pub working_dir: String,
    pub container_name: Option<String>,
}

pub fn command(
    input: &EngineInput,
    binary: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Command {
    let mut cmd = if let Some(sandbox) = &input.command_sandbox {
        let mut cmd = Command::new(&sandbox.program);
        let mut prefix = sandbox.prefix_args.clone();
        for (key, value) in env {
            if sandbox.env_whitelist.contains(key) {
                let extra = if sandbox.program == "bwrap" {
                    vec!["--setenv".into(), key.clone(), value.clone()]
                } else {
                    vec!["--env".into(), format!("{key}={value}")]
                };
                if sandbox.program == "docker" {
                    let image_position = prefix.len().saturating_sub(1);
                    prefix.splice(image_position..image_position, extra);
                } else {
                    prefix.extend(extra);
                }
            }
        }
        cmd.args(&prefix);
        // Allow explicitly selected standalone executables outside the system runtime mounts.
        if sandbox.program == "bwrap" && std::path::Path::new(binary).is_absolute() {
            if let Ok(source) = std::fs::canonicalize(binary) {
                cmd.arg("--ro-bind").arg(source).arg(binary);
            }
        }
        if sandbox.program == "bwrap" {
            cmd.arg("--");
        }
        cmd.arg(binary).args(args);
        cmd.env_clear();
        for key in sandbox
            .env_whitelist
            .iter()
            .map(String::as_str)
            .chain(["PATH", "LANG", "HOME"])
        {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
        for (key, value) in env {
            if sandbox.env_whitelist.contains(key) {
                cmd.env(key, value);
            }
        }
        cmd
    } else {
        let mut cmd = Command::new(binary);
        cmd.args(args).envs(env);
        cmd
    };
    if !input.working_dir.is_empty() {
        cmd.current_dir(&input.working_dir);
    }
    cmd
}

#[cfg(unix)]
struct ProcessGroup(u32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        use nix::{
            sys::signal::{killpg, Signal},
            unistd::Pid,
        };
        let _ = killpg(Pid::from_raw(self.0 as i32), Signal::SIGKILL);
    }
}

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024; // 10MB limit

async fn read_stream(
    stream: impl AsyncRead + Unpin,
    callback: Option<StreamCallback>,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    let mut reader = BufReader::new(stream);
    let mut output = Vec::new();
    let mut chunk = Vec::new();
    let mut truncated = false;
    loop {
        chunk.clear();
        if reader.read_until(b'\n', &mut chunk).await? == 0 {
            break;
        }
        if let Some(cb) = &callback {
            cb(&String::from_utf8_lossy(&chunk));
        }
        if output.len() + chunk.len() <= max_bytes {
            output.extend_from_slice(&chunk);
        } else if !truncated {
            let remaining = max_bytes.saturating_sub(output.len());
            if remaining > 0 {
                output.extend_from_slice(&chunk[..remaining]);
            }
            output.extend_from_slice(b"\n...[OUTPUT TRUNCATED: buffer exceeded limit]...\n");
            truncated = true;
        }
    }
    Ok(output)
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
    let group = ProcessGroup(child.id().expect("spawned child has a PID"));
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let run = async {
        let (status, stdout, stderr) = tokio::try_join!(
            child.wait(),
            read_stream(stdout, callback, max_output_bytes),
            read_stream(stderr, None, max_output_bytes)
        )?;
        Ok::<_, std::io::Error>(std::process::Output {
            status,
            stdout,
            stderr,
        })
    };
    match tokio::time::timeout(timeout, run).await {
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
    let cleanup = ContainerCleanup(
        input
            .command_sandbox
            .as_ref()
            .and_then(|sandbox| sandbox.container_name.clone()),
    );
    let limit = input.max_output_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let result = run_process_with_limit(command(input, binary, args, env), timeout(input), callback, limit).await;
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

pub fn timeout(input: &EngineInput) -> Duration {
    Duration::from_secs(if input.timeout_seconds == 0 {
        300
    } else {
        input.timeout_seconds
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{AntigravityEngine, ClaudeCodeEngine, ComputeEngine, GenericCliEngine};
    use std::{path::PathBuf, sync::Arc};

    fn marker(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tuzygent-{label}-{}", uuid_for_test()))
    }
    fn uuid_for_test() -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn shell(script: String) -> GenericCliEngine {
        GenericCliEngine::new(
            "test",
            "/bin/sh",
            vec!["-c".into(), script],
            Default::default(),
            None,
        )
    }

    #[tokio::test]
    async fn drains_stderr_while_stdout_is_open() -> Result<(), Box<dyn std::error::Error>> {
        let engine = shell("i=0; while [ $i -lt 10000 ]; do printf 'a long diagnostic message to fill stderr\\n' >&2; i=$((i+1)); done; printf done".into());
        let mut input = EngineInput::new("", "/tmp");
        input.timeout_seconds = 5;
        let result = engine.execute(&input).await?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout_content, "done");
        Ok(())
    }

    #[tokio::test]
    async fn timeout_kills_descendants() {
        let path = marker("timeout");
        let engine = shell(format!(
            "(sleep 2; printf leaked > {}) & wait",
            path.display()
        ));
        let mut input = EngineInput::new("", "/tmp");
        input.timeout_seconds = 1;
        assert!(matches!(
            engine.execute(&input).await,
            Err(EngineError::Timeout)
        ));
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(!path.exists(), "child survived timeout");
    }

    #[tokio::test]
    async fn cancellation_kills_descendants() {
        let path = marker("cancel");
        let engine = shell(format!(
            "(sleep 2; printf leaked > {}) & wait",
            path.display()
        ));
        let task = tokio::spawn(async move { engine.execute(&EngineInput::new("", "/tmp")).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        task.abort();
        let _ = task.await;
        tokio::time::sleep(Duration::from_millis(2100)).await;
        assert!(!path.exists(), "child survived cancellation");
    }

    #[tokio::test]
    async fn every_cli_uses_the_wrapper_and_never_falls_back_to_host() -> Result<(), Box<dyn std::error::Error>> {
        let engines: Vec<Arc<dyn ComputeEngine>> = vec![
            Arc::new(shell("exit 0".into())),
            Arc::new(ClaudeCodeEngine::new(Some("/bin/true".into()), None)),
            Arc::new(AntigravityEngine::new(Some("/bin/true".into()), None)),
            Arc::new(crate::CodexEngine::new(Some("/bin/true".into()), None)),
        ];
        for engine in engines {
            let mut input = EngineInput::new("probe", "/tmp");
            input.command_sandbox = Some(CommandSandbox {
                program: "/bin/sh".into(),
                prefix_args: vec!["-c".into(), "exit 42".into(), "wrapper".into()],
                env_whitelist: vec![],
                working_dir: "/workspace".into(),
                container_name: None,
            });
            assert_eq!(engine.execute(&input).await?.exit_code, 42);
            input.command_sandbox.as_mut().ok_or("missing command_sandbox")?.program = "/missing-tuzygent-sandbox".into();
            assert!(engine.execute(&input).await.is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn antigravity_enforces_timeout_and_drains_stderr() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;
        let script = marker("agy-script");
        std::fs::write(&script, "#!/bin/sh\nsleep 2\n")?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
        let engine = AntigravityEngine::new(Some(script.to_string_lossy().into()), None);
        let mut input = EngineInput::new("test", "/tmp");
        input.timeout_seconds = 1;
        assert!(matches!(
            engine.execute(&input).await,
            Err(EngineError::Timeout)
        ));
        std::fs::write(&script, "#!/bin/sh\ni=0; while [ $i -lt 10000 ]; do printf 'diagnostic diagnostic diagnostic\\n' >&2; i=$((i+1)); done\nprintf '%s\\n' '{\"event\":\"result\",\"result\":{\"response\":\"ok\"}}'\n")?;
        input.timeout_seconds = 5;
        assert_eq!(engine.execute(&input).await?.stdout_content, "ok");
        std::fs::remove_file(script)?;
        Ok(())
    }
}
