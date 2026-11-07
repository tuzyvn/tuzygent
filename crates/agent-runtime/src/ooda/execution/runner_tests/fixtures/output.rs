use agent_engine::{EngineKind, EngineOutput};

pub(super) fn mock_remediation(id: &str, title: &str, cmd: &str) -> EngineOutput {
    let rem_json = serde_json::json!({
        "id": id,
        "title": title,
        "command_or_prompt": cmd,
        "expected_verification": "file: verified_fixture"
    });
    EngineOutput {
        stdout_content: rem_json.to_string(),
        exit_code: 0,
        execution_duration_ms: 10,
        engine_used: EngineKind::Mock,
        session_id: None,
    }
}

pub(super) fn mock_ok(stdout: &str) -> EngineOutput {
    EngineOutput {
        stdout_content: stdout.to_string(),
        exit_code: 0,
        execution_duration_ms: 10,
        engine_used: EngineKind::Mock,
        session_id: None,
    }
}
