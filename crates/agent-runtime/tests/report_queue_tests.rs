use agent_runtime::{TerminalReportKind, TerminalReportStore, TerminalTaskReport};
use chrono::Utc;
use std::path::{Path, PathBuf};
use uuid::Uuid;

struct TestWorkspace(PathBuf);

impl TestWorkspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("report-queue-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn report(task_id: &str) -> TerminalTaskReport {
    TerminalTaskReport {
        version: 1,
        created_at: Utc::now(),
        kind: TerminalReportKind::Completed,
        task_id: task_id.to_owned(),
        agent_id: "agent-test".to_owned(),
        output: Some("finished".to_owned()),
        error_message: None,
        retry_count: 0,
        failure_reason: None,
    }
}

#[tokio::test]
async fn acknowledge_removes_only_the_matching_pending_report_and_is_idempotent() {
    let workspace = TestWorkspace::new();
    let store = TerminalReportStore::new(workspace.path());
    store.enqueue(&report("task-ack")).await.unwrap();
    store.enqueue(&report("task-keep")).await.unwrap();

    store.acknowledge("task-ack").await.unwrap();
    store.acknowledge("task-ack").await.unwrap();

    let pending = store.list_pending().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].task_id, "task-keep");
}

#[tokio::test]
async fn list_pending_reads_reports_and_ignores_non_report_files_and_invalid_json() {
    let workspace = TestWorkspace::new();
    let store = TerminalReportStore::new(workspace.path());
    let queued_path = store.enqueue(&report("task-pending")).await.unwrap();
    let queue_dir = queued_path.parent().unwrap();
    tokio::fs::write(queue_dir.join("broken.json"), b"not valid json")
        .await
        .unwrap();
    tokio::fs::write(queue_dir.join("in-progress.json.tmp"), b"partial")
        .await
        .unwrap();

    let pending = store.list_pending().await.unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].task_id, "task-pending");
    assert_eq!(pending[0].agent_id, "agent-test");
    assert_eq!(pending[0].output.as_deref(), Some("finished"));
}

#[tokio::test]
async fn quarantine_moves_report_out_of_pending_and_is_idempotent_for_missing_reports() {
    let workspace = TestWorkspace::new();
    let store = TerminalReportStore::new(workspace.path());
    let queued_path = store.enqueue(&report("task-quarantine")).await.unwrap();
    let quarantined_path = queued_path
        .parent()
        .unwrap()
        .join("quarantined")
        .join(queued_path.file_name().unwrap());

    store.quarantine("task-quarantine").await.unwrap();

    assert!(!queued_path.exists());
    assert!(quarantined_path.is_file());
    assert!(store.list_pending().await.unwrap().is_empty());

    store.quarantine("task-quarantine").await.unwrap();
    store.quarantine("unknown-task").await.unwrap();
    assert!(quarantined_path.is_file());
}
