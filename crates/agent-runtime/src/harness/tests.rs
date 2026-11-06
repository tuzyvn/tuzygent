use super::*;
use uuid::Uuid;

#[tokio::test]
async fn corrupt_lessons_are_not_overwritten() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("harness-corrupt-{}", Uuid::new_v4()));
    let store = HarnessMemoryStore::new(dir.to_str().ok_or("invalid path")?);
    tokio::fs::write(&store.memory_path, b"invalid json").await?;
    assert!(store.record_lesson("agent", "a", "error", "fix").await.is_err());
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
            store.record_lesson("agent", &i.to_string(), "error", "fix").await
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
