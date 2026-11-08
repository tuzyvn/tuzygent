use super::*;
use uuid::Uuid;

fn lesson(id: &str) -> HarnessLesson {
    HarnessLesson {
        id: id.to_string(),
        node_type: "agent".to_string(),
        identifier: format!("identifier-{id}"),
        error_pattern: format!("error-{id}"),
        remediation_strategy: format!("fix-{id}"),
        success_count: 0,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        last_applied_at: None,
    }
}

#[test]
fn lesson_cache_with_capacity_accepts_and_retrieves_lessons() {
    let mut cache = HarnessLessonCache::with_capacity(2);
    let first = lesson("first");
    let second = lesson("second");
    let third = lesson("third");

    cache.insert(&first);
    cache.insert(&second);
    cache.insert(&third);

    assert_eq!(
        cache.get("first").map(|stored| stored.id.as_str()),
        Some("first")
    );
    assert_eq!(
        cache.get("second").map(|stored| stored.id.as_str()),
        Some("second")
    );
    assert_eq!(
        cache.get("third").map(|stored| stored.id.as_str()),
        Some("third")
    );
}

#[test]
fn lesson_cache_evicts_oldest_entries_only_when_over_the_limit() {
    let mut cache = HarnessLessonCache::new(Some(4));
    let lessons = [
        lesson("first"),
        lesson("second"),
        lesson("third"),
        lesson("fourth"),
    ];
    for item in &lessons {
        cache.insert(item);
    }

    cache.set_max_capacity(2);

    assert!(cache.get("first").is_none());
    assert!(cache.get("second").is_none());
    assert_eq!(
        cache.get("third").map(|stored| stored.id.as_str()),
        Some("third")
    );
    assert_eq!(
        cache.get("fourth").map(|stored| stored.id.as_str()),
        Some("fourth")
    );

    cache.evict_if_needed();

    assert_eq!(
        cache.get("third").map(|stored| stored.id.as_str()),
        Some("third")
    );
    assert_eq!(
        cache.get("fourth").map(|stored| stored.id.as_str()),
        Some("fourth")
    );
}

#[test]
fn lesson_cache_with_zero_limit_does_not_retain_lessons() {
    let mut cache = HarnessLessonCache::new(Some(0));
    let item = lesson("zero-limit");

    cache.insert(&item);
    cache.evict_if_needed();

    assert!(cache.get("zero-limit").is_none());
}

#[tokio::test]
async fn corrupt_lessons_are_not_overwritten() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("harness-corrupt-{}", Uuid::new_v4()));
    let store = HarnessMemoryStore::new(dir.to_str().ok_or("invalid path")?);
    tokio::fs::write(&store.memory_path, b"invalid json").await?;
    assert!(store
        .record_lesson("agent", "a", "error", "fix")
        .await
        .is_err());
    assert_eq!(tokio::fs::read(&store.memory_path).await?, b"invalid json");
    let _ = tokio::fs::remove_dir_all(dir).await;
    Ok(())
}

#[tokio::test]
async fn concurrent_writers_preserve_all_lessons() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("harness-concurrent-{}", Uuid::new_v4()));
    let mut tasks = Vec::new();
    for i in 0..32 {
        let store = HarnessMemoryStore::new(dir.to_str().ok_or("invalid path")?);
        tasks.push(tokio::spawn(async move {
            store
                .record_lesson("agent", &i.to_string(), "error", "fix")
                .await
        }));
    }
    for task in tasks {
        task.await??;
    }
    assert_eq!(
        HarnessMemoryStore::new(dir.to_str().ok_or("invalid path")?)
            .try_load_lessons()
            .await?
            .len(),
        32
    );
    let _ = tokio::fs::remove_dir_all(dir).await;
    Ok(())
}
