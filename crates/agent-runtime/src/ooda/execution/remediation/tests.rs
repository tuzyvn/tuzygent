use super::*;

#[test]
fn test_parse_remediation_json_codeblock() {
    let parent = SubTask::new("ST-1", "Run Build", "cargo build", "Exit code 0");
    let raw = r#"Analyzing error:
```json
{
  "id": "ST-1-FIX-1",
  "title": "Fix Missing Dependency",
  "command_or_prompt": "cargo add serde",
  "expected_verification": "Build passes"
}
```"#;

    let rem = SubtaskExecutor::parse_remediation_json(raw, &parent, "cannot find serde", 1)
        .expect("should parse remediation");
    assert_eq!(rem.id, "ST-1-FIX-1");
    assert_eq!(rem.title, "Fix Missing Dependency");
    assert_eq!(rem.command_or_prompt, "cargo add serde");
    assert_eq!(rem.expected_verification, "Build passes");
    assert_eq!(rem.parent_subtask_id.as_deref(), Some("ST-1"));
    assert_eq!(rem.retry_count, 1);
    assert_eq!(
        rem.remediation_for_error.as_deref(),
        Some("cannot find serde")
    );
    assert!(rem.is_remediation());
}

#[test]
fn test_parse_remediation_json_fallback_defaults() {
    let parent = SubTask::new("ST-2", "Run Lint", "cargo clippy", "Exit code 0");
    let raw = r#"{
  "title": "Fix clippy warning"
}"#;

    let rem = SubtaskExecutor::parse_remediation_json(raw, &parent, "clippy warning", 2)
        .expect("should parse remediation with fallbacks");
    assert_eq!(rem.id, "ST-2-FIX-2");
    assert_eq!(rem.title, "Fix clippy warning");
    assert_eq!(rem.command_or_prompt, "Apply remediation");
    assert_eq!(rem.expected_verification, "Error resolved");
}

#[test]
fn test_parse_remediation_json_invalid() {
    let parent = SubTask::new("ST-1", "Compile", "make", "ok");
    let raw = "no json here";
    assert!(SubtaskExecutor::parse_remediation_json(raw, &parent, "err", 1).is_none());
}

#[tokio::test]
async fn test_attempt_remediation_retry_exhausted() {
    use crate::ooda::execution::runner::QueueExecutionOutcome;
    use crate::supplement::InFlightSupplementController;
    use agent_domain::{GuardrailPolicy, Priority, TaskStatus};
    use agent_engine::cli::MockEngine;
    use agent_memory::{DurableMemoryStore, ScratchpadManager};
    use std::sync::Arc;

    let temp_dir = std::env::temp_dir().join(format!("test_rem_exhaust_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);

    let executor = SubtaskExecutor::new(
        Arc::new(MockEngine::new("")),
        GuardrailPolicy {
            max_retries_on_error: 0,
            ..Default::default()
        },
        ScratchpadManager::new(temp_dir.join("scratch")),
        DurableMemoryStore::new(temp_dir.join("durable.db")),
        temp_dir.to_string_lossy().to_string(),
        InFlightSupplementController::new(1),
        None,
        None,
        30,
    );

    let mut task = Task::new("Exhaust Task", "Desc", Priority::Normal);
    let st = SubTask::new("ST-1", "Failing step", "exit 1", "ok");
    task.subtasks.push(st.clone());

    let mut total_spawned = 0;
    let outcome = attempt_remediation_retry(
        &executor,
        &mut task,
        0,
        st,
        "Error details",
        RemediationFailureKind::ExitCode,
        &temp_dir,
        "ctx",
        &mut total_spawned,
    )
    .await;

    assert!(matches!(
        outcome,
        RemediationOutcome::Failed(QueueExecutionOutcome::Failed { error }) if error == "Error details"
    ));
    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::Failed { .. }));

    let _ = std::fs::remove_dir_all(&temp_dir);
}
