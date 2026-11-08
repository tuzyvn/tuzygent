use agent_engine::{EngineInput, EngineKind, EngineRouter, MockEngine};
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
