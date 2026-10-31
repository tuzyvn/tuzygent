#![cfg(unix)]
use std::{os::unix::fs::PermissionsExt, process::Command};
#[test]
fn run_command_delegates_to_configured_codex() {
    let directory =
        std::env::temp_dir().join(format!("tuzygent-cli-codex-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("codex");
    std::fs::write(
        &binary,
        include_str!("../../agent-engine/tests/fixtures/codex.sh"),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        directory.join("tuzygent.toml"),
        format!(
            r#"
        [defaults]
        engine = "codex"
        sandbox_driver = "host"
        timeout_seconds = 5
        [engines.codex]
        kind = "codex"
        binary_path = {:?}
        [engines.codex.env]
        CODEX_TEST_CASE = "ooda"
    "#,
            binary.to_str().unwrap()
        ),
    )
    .unwrap();
    for explicit in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-ctl"));
        command.args([
            "run",
            "--agent-id",
            "dev",
            "--goal",
            "delegate task",
            "--workspace",
            directory.to_str().unwrap(),
        ]);
        if explicit {
            command.args(["--engine", "codex"]);
        }
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("Using engine: codex"), "{stdout}");
        assert!(stdout.contains("Completed"), "{stdout}");
        assert!(!stdout.contains("Defaulting to Antigravity"));
    }
    let config = std::fs::read_to_string(directory.join("tuzygent.toml"))
        .unwrap()
        .replace("CODEX_TEST_CASE = \"ooda\"", "CODEX_TEST_CASE = \"exit\"");
    std::fs::write(directory.join("tuzygent.toml"), config).unwrap();
    let failed = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "run",
            "--agent-id",
            "dev",
            "--goal",
            "delegate task",
            "--engine",
            "codex",
            "--workspace",
            directory.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !failed.status.success(),
        "failed delegation returned success"
    );
    assert!(String::from_utf8_lossy(&failed.stderr).contains("Mission did not complete"));
    std::fs::remove_dir_all(directory).unwrap();
}
