use agent_engine::{EngineError, EngineInput, EngineKind};
use std::sync::{Arc, Mutex};

#[test]
fn is_auth_or_quota_matches_auth_and_quota_markers_case_insensitively() {
    let auth_messages = [
        "authentication failed",
        "401 Unauthorized",
        "403 Forbidden",
        "RATE LIMIT reached",
        "rate_limit exceeded",
        "quota exhausted",
        "credentials exhausted",
    ];

    for message in auth_messages {
        assert!(
            EngineError::ApiRequestFailed(message.into()).is_auth_or_quota(),
            "expected {message:?} to be classified as an auth/quota error"
        );
    }

    for error in [
        EngineError::ProcessFailed("command not found".into()),
        EngineError::ApiRequestFailed("503 service unavailable".into()),
        EngineError::Timeout,
        EngineError::PolicyRejected("blocked by policy".into()),
    ] {
        assert!(
            !error.is_auth_or_quota(),
            "unexpected classification: {error}"
        );
    }
}

#[test]
fn from_name_maps_canonical_names_aliases_and_unknown_names() {
    let known_names = [
        ("Claude", EngineKind::ClaudeCode),
        ("claude_code", EngineKind::ClaudeCode),
        ("ClaudeCode", EngineKind::ClaudeCode),
        ("ANTIGRAVITY", EngineKind::Antigravity),
        ("agy", EngineKind::Antigravity),
        ("Codex", EngineKind::Codex),
        ("custom", EngineKind::CustomAgentApi),
        ("CUSTOM_API", EngineKind::CustomAgentApi),
        ("Mock", EngineKind::Mock),
    ];

    for (name, expected) in known_names {
        assert_eq!(EngineKind::from_name(name), expected, "input: {name}");
    }

    assert_eq!(
        EngineKind::from_name("MySpecialEngine"),
        EngineKind::Generic("myspecialengine".into())
    );
    assert_eq!(
        EngineKind::from_name(""),
        EngineKind::Generic(String::new())
    );
}

#[test]
fn with_stream_callback_invokes_callback_and_skips_it_in_serialization() {
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let callback_chunks = Arc::clone(&chunks);
    let input = EngineInput::new("prompt", "/tmp").with_stream_callback(Arc::new(move |chunk| {
        callback_chunks.lock().unwrap().push(chunk.to_owned());
    }));

    let callback = input
        .on_stream_chunk
        .as_ref()
        .expect("builder should store the stream callback");
    callback("first chunk");
    callback("second chunk");
    assert_eq!(*chunks.lock().unwrap(), ["first chunk", "second chunk"]);

    let serialized = serde_json::to_value(&input).unwrap();
    assert!(serialized.get("on_stream_chunk").is_none());
    let restored: EngineInput = serde_json::from_value(serialized).unwrap();
    assert!(restored.on_stream_chunk.is_none());
}
