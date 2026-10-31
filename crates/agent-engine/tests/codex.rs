#![cfg(unix)]
use agent_engine::{
    CodexEngine, ComputeEngine, EngineError, EngineInput, EngineKind, EnginesConfig,
};
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tuzygent-codex-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let script = path.join("codex");
        std::fs::write(&script, include_str!("fixtures/codex.sh")).unwrap();
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn engine(&self, case: &str) -> CodexEngine {
        CodexEngine::new(
            Some(self.0.join("codex").to_str().unwrap().into()),
            Some("configured-model".into()),
        )
        .with_env(HashMap::from([
            ("CODEX_TEST_CASE".into(), case.into()),
            (
                "CODEX_TEST_ARGS".into(),
                self.0.join("argv").to_str().unwrap().into(),
            ),
        ]))
    }
    fn input(&self) -> EngineInput {
        EngineInput::new(
            "--task with 'quotes' $(not-a-command)",
            self.0.to_str().unwrap(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test]
async fn delegates_with_literal_prompt_and_streams_events_and_final_message() {
    let fixture = Fixture::new();
    let mut input = fixture.input();
    input.system_context = "context from supervisor".into();
    let streamed = Arc::new(Mutex::new(String::new()));
    let callback = streamed.clone();
    input.on_stream_chunk = Some(Arc::new(move |chunk| {
        callback.lock().unwrap().push_str(chunk)
    }));
    let events = Arc::new(Mutex::new(vec![]));
    let callback = events.clone();
    input.on_event = Some(Arc::new(move |event| {
        callback
            .lock()
            .unwrap()
            .push(event["type"].as_str().unwrap().to_owned())
    }));
    let output = fixture.engine("success").execute(&input).await.unwrap();
    assert_eq!(output.engine_used, EngineKind::Codex);
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout_content, "CODEX_DELEGATED");
    assert_eq!(
        output.session_id.as_deref(),
        Some("12345678-1234-4234-8234-123456789abc")
    );
    assert_eq!(*streamed.lock().unwrap(), "CODEX_DELEGATED\n");
    assert!(events.lock().unwrap().contains(&"turn.completed".into()));
    let args = std::fs::read_to_string(fixture.0.join("argv")).unwrap();
    assert!(args.starts_with("exec\n--json\n--sandbox\nworkspace-write\n"));
    assert!(args.contains("approval_policy=\"never\"\n"));
    assert!(args.contains("--model\nconfigured-model\n--\n[INSTRUCTIONS]\ncontext from supervisor"));
    assert!(args.contains("--task with 'quotes' $(not-a-command)"));
    assert!(!args.contains("dangerously-bypass"));
}
#[tokio::test]
async fn detects_protocol_failures_even_when_cli_exits_zero() {
    let fixture = Fixture::new();
    for (case, reason) in [
        ("fail", "model rejected"),
        ("invalid", "invalid JSONL"),
        ("empty", "without a final"),
        ("partial", "turn.completed"),
        ("exit", "authentication failed"),
    ] {
        let output = fixture
            .engine(case)
            .execute(&fixture.input())
            .await
            .unwrap();
        assert_ne!(output.exit_code, 0, "{case}");
        assert!(
            output.stdout_content.contains(reason),
            "{case}: {}",
            output.stdout_content
        );
    }
    assert_eq!(
        fixture
            .engine("transient")
            .execute(&fixture.input())
            .await
            .unwrap()
            .exit_code,
        0
    );
}
#[tokio::test]
async fn resume_requires_an_explicit_session_and_keeps_sandbox_policy() {
    let fixture = Fixture::new();
    let engine = fixture.engine("success");
    let id = "12345678-1234-4234-8234-123456789abc";
    let output = engine.resume(&fixture.input(), id).await.unwrap();
    assert_eq!(output.exit_code, 0);
    let args = std::fs::read_to_string(fixture.0.join("argv")).unwrap();
    assert!(args.contains(&format!("resume\n{id}\n--\n")));
    assert!(!args.contains("--last"));
    assert!(engine.resume(&fixture.input(), "--last").await.is_err());
}
#[tokio::test]
async fn timeout_is_enforced() {
    let fixture = Fixture::new();
    let mut input = fixture.input();
    input.timeout_seconds = 1;
    assert!(matches!(
        fixture.engine("timeout").execute(&input).await,
        Err(EngineError::Timeout)
    ));
}
#[tokio::test]
async fn named_codex_configs_keep_their_own_binary_and_model() {
    let fixture = Fixture::new();
    let mut config = EnginesConfig::default();
    let single = config.engines.get_mut("codex").unwrap();
    single.binary_path = Some(fixture.0.join("codex").to_str().unwrap().into());
    let mut other = single.clone();
    other.binary_path = Some("/missing-codex".into());
    config.engines.insert("other_codex".into(), other);
    let router = agent_engine::EngineRouter::from_config(&config);
    assert_eq!(
        router
            .dispatch_by_name(Some("codex"), &fixture.input())
            .await
            .unwrap()
            .engine_used,
        EngineKind::Codex
    );
    assert!(router
        .dispatch_by_name(Some("other_codex"), &fixture.input())
        .await
        .is_err());
    assert!(agent_engine::configured_engine(&config, "typo-engine", None).is_err());
    let mut invalid = config.clone();
    let mut valid_alias = invalid.engines["codex"].clone();
    valid_alias.kind = "codex".into();
    invalid.engines.insert("valid_codex".into(), valid_alias);
    invalid.engines.get_mut("codex").unwrap().kind = "unsupported-kind".into();
    let router = agent_engine::EngineRouter::from_config(&invalid);
    assert!(router
        .dispatch_by_name(Some("codex"), &fixture.input())
        .await
        .is_err());
}
