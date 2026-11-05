use super::*;
use crate::{
    AntigravityEngine, ClaudeCodeEngine, ComputeEngine, EngineError, EngineInput, GenericCliEngine,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[tokio::test]
async fn oversized_unterminated_line_returns_error() {
    let mut command = tokio::process::Command::new("sh");
    command.arg("-c").arg("printf '12345678901234567890'");
    let result =
        run_process_with_limit(command, Duration::from_secs(2), None, 8).await;
    assert!(
        matches!(result, Err(EngineError::ProcessFailed(message)) if message.contains("output line exceeds"))
    );
}

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
async fn every_cli_uses_the_wrapper_and_never_falls_back_to_host(
) -> Result<(), Box<dyn std::error::Error>> {
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
        input
            .command_sandbox
            .as_mut()
            .ok_or("missing command_sandbox")?
            .program = "/missing-tuzygent-sandbox".into();
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
    std::fs::write(
        &script,
        "#!/bin/sh\ni=0; while [ $i -lt 10000 ]; do printf 'diagnostic diagnostic diagnostic\\n' >&2; i=$((i+1)); done\nprintf '%s\\n' '{\"event\":\"result\",\"result\":{\"response\":\"ok\"}}'\n",
    )?;
    input.timeout_seconds = 5;
    assert_eq!(engine.execute(&input).await?.stdout_content, "ok");
    std::fs::remove_file(script)?;
    Ok(())
}
