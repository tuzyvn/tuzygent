use agent_domain::{Priority, SubTask, Task, TaskStatus};
use agent_memory::DurableMemoryStore;

#[test]
fn test_subtask_remediation_model() {
    let parent = SubTask::new(
        "ST-1",
        "Initial Probe",
        "check service status",
        "service is up",
    );
    assert_eq!(parent.retry_count, 0);
    assert!(!parent.is_remediation());

    let remediation = SubTask::new_remediation(
        "ST-1-FIX-1",
        "Remediate Connection Timeout",
        "restart proxy service",
        &parent.id,
        "Connection refused on port 8085",
        1,
    );

    assert!(remediation.is_remediation());
    assert_eq!(remediation.parent_subtask_id.as_deref(), Some("ST-1"));
    assert_eq!(remediation.retry_count, 1);
    assert!(remediation.remediation_for_error.is_some());
}

#[tokio::test]
async fn test_durable_memory_semantic_filter() {
    let temp_dir = std::env::temp_dir().join(format!("test_mem_{}", uuid::Uuid::new_v4().simple()));
    let store = DurableMemoryStore::new(&temp_dir);

    store.save_lesson("Nginx port 8085 requires proxy_pass header configuration").await.unwrap();
    store.save_lesson("Rust async tokio channels should specify bounded capacity").await.unwrap();
    store.save_lesson("Database migrations must run before web server startup").await.unwrap();

    let nginx_context = store.load_relevant_context("Nginx proxy error 502").await.unwrap();
    assert!(nginx_context.contains("Nginx port 8085"));

    let rust_context = store.load_relevant_context("Rust async worker pool").await.unwrap();
    assert!(rust_context.contains("Rust async tokio"));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[test]
fn test_task_status_lifecycle() {
    let mut task = Task::new("Deploy Microservice", "Deploy service to k8s cluster", Priority::High);
    assert_eq!(task.status, TaskStatus::Queued);
    assert!(task.subtasks.is_empty());

    let st1 = SubTask::new("ST-1", "Lint configs", "kustomize build", "Zero lint errors");
    task.subtasks.push(st1);
    assert_eq!(task.subtasks.len(), 1);
}

#[tokio::test]
async fn test_structured_lesson_persistence() {
    let temp_dir = std::env::temp_dir().join(format!("test_struct_mem_{}", uuid::Uuid::new_v4().simple()));
    let store = DurableMemoryStore::new(&temp_dir);

    let lesson = agent_memory::StructuredLesson {
        id: "lesson_test_101".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        task_title: "Fix Port Conflict".to_string(),
        trigger_error: Some("Address already in use: 3300".to_string()),
        remediation_applied: Some("fuser -k 3300/tcp".to_string()),
        general_insight: "Always check existing listeners before binding".to_string(),
        success: true,
    };

    store.save_structured_lesson(&lesson).await.unwrap();

    let loaded = store.load_structured_lessons().await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "lesson_test_101");
    assert_eq!(loaded[0].task_title, "Fix Port Conflict");
    assert!(loaded[0].success);

    // Verify MEMORY.md was also updated
    let mem_context = store.load_relevant_context("Port Conflict").await.unwrap();
    assert!(mem_context.contains("Fix Port Conflict"));
    assert!(mem_context.contains("Always check existing listeners"));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}
