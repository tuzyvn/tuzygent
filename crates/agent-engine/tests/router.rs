use agent_engine::{
    configured_engine, EngineInput, EngineKind, EngineRouter, EnginesConfig, MockEngine,
    SingleEngineConfig,
};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn register_adds_engine_for_dispatch_and_kind_name_resolution() {
    let mut router = EngineRouter::new(EngineKind::Mock);
    router.register(Arc::new(MockEngine::new("registered")));

    assert_eq!(router.resolve_kind(Some("MOCK")), EngineKind::Mock);

    let output = router
        .dispatch(None, &EngineInput::new("task", "."))
        .await
        .unwrap();
    assert_eq!(output.engine_used, EngineKind::Mock);
    assert_eq!(output.stdout_content, "registered: task");
}

#[tokio::test]
async fn dispatch_uses_explicitly_preferred_engine() {
    let mut router = EngineRouter::new(EngineKind::Codex);
    router.register(Arc::new(MockEngine::new("preferred")));

    let output = router
        .dispatch(Some(EngineKind::Mock), &EngineInput::new("task", "."))
        .await
        .unwrap();

    assert_eq!(output.engine_used, EngineKind::Mock);
    assert_eq!(output.stdout_content, "preferred: task");
}

#[tokio::test]
async fn register_named_allows_case_insensitive_name_dispatch() {
    let mut router = EngineRouter::new(EngineKind::Codex);
    router.register_named("my-agent", Arc::new(MockEngine::new("named")));

    assert_eq!(router.resolve_kind(Some("MY-AGENT")), EngineKind::Mock);

    let output = router
        .dispatch_by_name(Some("MY-AGENT"), &EngineInput::new("task", "."))
        .await
        .unwrap();
    assert_eq!(output.engine_used, EngineKind::Mock);
    assert_eq!(output.stdout_content, "named: task");
}

#[test]
fn resolve_kind_handles_default_alias_and_unknown_names() {
    let router = EngineRouter::new(EngineKind::Codex);

    assert_eq!(router.resolve_kind(None), EngineKind::Codex);
    assert_eq!(router.resolve_kind(Some("AGY")), EngineKind::Antigravity);
    assert_eq!(
        router.resolve_kind(Some("Custom_API")),
        EngineKind::CustomAgentApi
    );
    assert_eq!(
        router.resolve_kind(Some("future-engine")),
        EngineKind::Generic("future-engine".into())
    );
}

#[tokio::test]
async fn configured_engine_builds_the_engine_kind_declared_in_config() {
    let mut config = EnginesConfig::default();
    config.engines.insert(
        "test-agent".into(),
        SingleEngineConfig {
            kind: "mock".into(),
            binary_path: None,
            args: vec![],
            model: None,
            interactive_fallback: None,
            output_parser: None,
            env: HashMap::new(),
        },
    );

    let engine = configured_engine(&config, "test-agent", None).unwrap();
    let output = engine
        .execute(&EngineInput::new("configured task", "."))
        .await
        .unwrap();

    assert_eq!(output.engine_used, EngineKind::Mock);
    assert_eq!(
        output.stdout_content,
        "Mock completed task: configured task"
    );
}
