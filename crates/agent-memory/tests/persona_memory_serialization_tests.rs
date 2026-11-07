use agent_memory::persona_memory::{MemoryRecord, RetrievedContext};

#[test]
fn memory_record_serialization_preserves_public_json_shape() {
    let expected = serde_json::json!({
        "id": "memory-1",
        "persona_id": "persona-1",
        "kind": "knowledge",
        "scope": "persona",
        "status": "active",
        "content": "Use a transaction",
        "conditions": "when updating multiple rows",
        "source": "task-1",
        "verified": true,
        "profile_version": 7,
        "created_at": "2026-11-07T12:00:00Z"
    });

    let record: MemoryRecord = serde_json::from_value(expected.clone()).unwrap();
    let serialized = serde_json::to_value(&record).unwrap();

    assert_eq!(serialized, expected);
    let decoded: MemoryRecord = serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}

#[test]
fn retrieved_context_serialization_preserves_public_json_shape() {
    let expected = serde_json::json!({
        "memory_ids": ["memory-1", "memory-2"],
        "text": "Retrieved memory context"
    });

    let context: RetrievedContext = serde_json::from_value(expected.clone()).unwrap();
    let serialized = serde_json::to_value(&context).unwrap();

    assert_eq!(serialized, expected);
    let decoded: RetrievedContext = serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}
