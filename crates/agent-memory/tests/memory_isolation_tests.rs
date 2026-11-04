use agent_memory::{ContextBuilder, DurableMemoryStore, ScratchpadManager, StructuredLesson};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(100);

fn unique_temp_dir() -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("agentcore_mem_iso_{}_{}", ts, id));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[tokio::test]
async fn test_structured_and_harness_lessons_coexist_without_data_loss() {
    let ws = unique_temp_dir();
    let durable = DurableMemoryStore::new(&ws);

    let structured = StructuredLesson {
        id: "struct-1".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        task_title: "Build System".to_string(),
        trigger_error: Some("linker error".to_string()),
        remediation_applied: Some("add -lpthread".to_string()),
        general_insight: "Always check pthread linkage".to_string(),
        success: true,
    };

    durable.save_structured_lesson(&structured).await.unwrap();

    let loaded = durable.load_structured_lessons().await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "struct-1");

    // Verify structured_lessons.json exists
    let struct_file = ws.join("storage").join("harness_memory").join("structured_lessons.json");
    assert!(struct_file.exists());

    // Clean up
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_corrupt_json_creates_backup_and_preserves_disk() {
    let ws = unique_temp_dir();
    let storage_dir = ws.join("storage").join("harness_memory");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let struct_file = storage_dir.join("structured_lessons.json");

    // Write malformed JSON
    std::fs::write(&struct_file, b"{ invalid json content here !!!").unwrap();

    let durable = DurableMemoryStore::new(&ws);
    let res = durable.load_structured_lessons().await;
    assert!(res.is_err(), "Corrupt JSON must return error instead of silently returning empty vec");

    // Verify a backup corrupt file was generated
    let entries = std::fs::read_dir(&storage_dir).unwrap();
    let has_corrupt_backup = entries.filter_map(|e| e.ok()).any(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        name.starts_with("structured_lessons.corrupt.")
    });
    assert!(has_corrupt_backup, "Corrupt file backup should be preserved");

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_task_scoped_scratchpad_isolation() {
    let ws = unique_temp_dir();
    let pad1 = ScratchpadManager::for_task(&ws, "task-101");
    let pad2 = ScratchpadManager::for_task(&ws, "task-102");

    pad1.append_log("Task 101 step 1 completed").await.unwrap();
    pad2.append_log("Task 102 step 1 completed").await.unwrap();

    let content1 = pad1.read().await.unwrap();
    let content2 = pad2.read().await.unwrap();

    assert!(content1.contains("Task 101"));
    assert!(!content1.contains("Task 102"));
    assert!(content2.contains("Task 102"));
    assert!(!content2.contains("Task 101"));

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_context_builder_character_bounds() {
    let builder = ContextBuilder::new("Implement high-throughput gateway")
        .with_lessons(vec!["Lesson 1: Always validate model whitelists".to_string()])
        .with_recent_logs(vec!["Log: Subtask 1 verified exit 0".to_string()])
        .add_subtask_step("ST-1".to_string(), "Created initial scaffolding".to_string())
        .with_artifacts(vec!["target/release/server".to_string()])
        .with_max_total_chars(5000);

    let (sys, work) = builder.build();
    assert!(sys.contains("PRIMARY GOAL"));
    assert!(sys.contains("Implement high-throughput gateway"));
    assert!(sys.contains("Lesson 1: Always validate model whitelists"));
    assert!(work.contains("RECENT SCRATCHPAD ACTIVITY"));
    assert!(work.contains("PREVIOUS SUBTASK OUTPUTS"));
    assert!(work.contains("CREATED ARTIFACTS"));
    assert!(sys.len() + work.len() <= 5000);
}
