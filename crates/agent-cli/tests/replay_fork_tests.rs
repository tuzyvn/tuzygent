use std::path::PathBuf;
use std::process::Command;

use agent_cli::cli::{Cli, Commands, OutputFormat};
use agent_cli::commands::replay::{handle, handle_fork};
use agent_cli::{
    execute_replay_or_fork, inspect_checkpoints, CheckpointListPayload, ForkArgs, ReplayArgs,
    ReplayOutcomePayload,
};
use agent_domain::{AgentId, Priority, SubTask, Task, TaskStatus};
use agent_runtime::{SqliteTaskStore, TaskStore};
use clap::Parser;

fn unique_test_workspace(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{}_{}", prefix, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp workspace");
    dir
}

async fn seed_three_step_task(workspace: &std::path::Path) -> Task {
    let store = SqliteTaskStore::new(workspace).expect("init sqlite store");
    let mut task = Task::new(
        "Database Migration Pipeline",
        "Migrate schema, transform rows, and validate checksums",
        Priority::High,
    );
    task.assigned_agent = Some(AgentId::from_str("agent-dba"));
    task.engine_name = Some("antigravity".to_string());
    task.status = TaskStatus::Failed {
        error: "Step 3 checksum mismatch".to_string(),
    };
    task.failure_count = 2;

    let mut st1 = SubTask::new("ST-1", "Backup & Schema Alter", "run step 1", "exit 0");
    st1.status = TaskStatus::Completed;
    st1.output_excerpt = Some("schema altered v2".to_string());

    let mut st2 = SubTask::new("ST-2", "Transform Records", "run step 2", "exit 0");
    st2.status = TaskStatus::Completed;
    st2.output_excerpt = Some("1000 rows transformed".to_string());

    let mut st3 = SubTask::new("ST-3", "Verify Checksums", "run step 3", "exit 0");
    st3.status = TaskStatus::Failed {
        error: "checksum failed on shard 2".to_string(),
    };
    st3.retry_count = 2;
    st3.output_excerpt = Some("shard 2 error".to_string());

    task.subtasks = vec![st1, st2, st3];

    store.save_task(&task).await.expect("save task");
    store
        .save_checkpoint(&task.id, &task.subtasks)
        .await
        .expect("save checkpoint");

    task
}

#[test]
fn test_cli_parse_replay_and_fork_commands() {
    // 1. Parse `agent-ctl replay <TASK_ID> --list --format json`
    let cli = Cli::try_parse_from([
        "agent-ctl",
        "replay",
        "TASK-12345",
        "--list",
        "--format",
        "json",
    ])
    .expect("parse replay --list");
    assert!(cli.validate().is_ok());
    match cli.command {
        Commands::Replay(args) => {
            assert_eq!(args.resolved_task_id(), Some("TASK-12345"));
            assert!(args.list);
            assert!(!args.fork);
            assert_eq!(args.format, OutputFormat::Json);
        }
        other => panic!("Expected Commands::Replay, got {:?}", other),
    }

    // 2. Parse `agent-ctl replay <TASK_ID> --from-step 2 --fork --goal "Updated context"`
    let cli = Cli::try_parse_from([
        "agent-ctl",
        "replay",
        "TASK-12345",
        "--from-step",
        "2",
        "--fork",
        "--goal",
        "Updated context",
    ])
    .expect("parse replay --from-step 2 --fork");
    assert!(cli.validate().is_ok());
    match cli.command {
        Commands::Replay(args) => {
            assert_eq!(args.resolved_task_id(), Some("TASK-12345"));
            assert_eq!(args.from_step, Some(2));
            assert!(args.fork);
            assert_eq!(args.goal.as_deref(), Some("Updated context"));
        }
        other => panic!("Expected Commands::Replay, got {:?}", other),
    }

    // 3. Parse top-level `agent-ctl fork <TASK_ID> --from-step 2 --title "Forked Pipeline"`
    let cli = Cli::try_parse_from([
        "agent-ctl",
        "fork",
        "TASK-12345",
        "--from-step",
        "2",
        "--title",
        "Forked Pipeline",
    ])
    .expect("parse fork --from-step 2");
    assert!(cli.validate().is_ok());
    match cli.command {
        Commands::Fork(args) => {
            assert_eq!(args.resolved_task_id(), Some("TASK-12345"));
            assert_eq!(args.from_step, Some(2));
            assert_eq!(args.title.as_deref(), Some("Forked Pipeline"));
        }
        other => panic!("Expected Commands::Fork, got {:?}", other),
    }

    // 4. Validation rejects empty task_id or from_step == 0
    let invalid_zero_step =
        Cli::try_parse_from(["agent-ctl", "replay", "TASK-12345", "--from-step", "0"])
            .expect("parse");
    assert!(invalid_zero_step.validate().is_err());
}

#[tokio::test]
async fn test_replay_task_from_specific_checkpoint_step_in_place() {
    let ws = unique_test_workspace("replay_in_place");
    let seeded = seed_three_step_task(&ws).await;
    let task_id_str = seeded.id.0.clone();

    // First verify checkpoint listing
    let list_payload = inspect_checkpoints(&ws, &task_id_str)
        .await
        .expect("inspect checkpoints");
    assert_eq!(list_payload.task_id, seeded.id);
    assert_eq!(list_payload.metadata.total_subtasks, 3);
    assert_eq!(list_payload.metadata.completed_subtasks, 2);
    assert_eq!(
        list_payload.metadata.last_completed_subtask_id.as_deref(),
        Some("ST-2")
    );

    // Replay task from step 2 (1-based):
    // - Step 1 ("ST-1") must remain Completed and keep its output_excerpt
    // - Step 2 ("ST-2") and Step 3 ("ST-3") must be rewound to Queued with cleared output_excerpt & retry_count
    // - Task ID remains the same, status becomes Queued, failure_count resets to 0
    let replay_args = ReplayArgs {
        task_id: Some(task_id_str.clone()),
        flag_id: None,
        list: false,
        from_step: Some(2),
        subtask_id: None,
        fork: false,
        title: None,
        goal: Some("Replay with fixed transformation rule".to_string()),
        prompt: Some("run step 2 --strict".to_string()),
        workspace: ws.clone(),
        format: OutputFormat::Json,
    };

    let outcome = execute_replay_or_fork(&replay_args)
        .await
        .expect("execute replay from step 2");

    assert_eq!(outcome.action, "replay");
    assert_eq!(outcome.source_task_id, seeded.id);
    assert_eq!(
        outcome.task.id, seeded.id,
        "Replay in-place must retain the same TaskId"
    );
    assert_eq!(outcome.task.status, TaskStatus::Queued);
    assert_eq!(outcome.task.failure_count, 0);
    assert_eq!(
        outcome.task.goal_description,
        "Replay with fixed transformation rule"
    );
    assert_eq!(outcome.from_step, Some(2));

    // Verify subtask states after replay from step 2
    assert_eq!(outcome.task.subtasks.len(), 3);
    assert_eq!(outcome.task.subtasks[0].id, "ST-1");
    assert_eq!(outcome.task.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(
        outcome.task.subtasks[0].output_excerpt.as_deref(),
        Some("schema altered v2")
    );

    assert_eq!(outcome.task.subtasks[1].id, "ST-2");
    assert_eq!(outcome.task.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(outcome.task.subtasks[1].output_excerpt, None);
    assert_eq!(
        outcome.task.subtasks[1].command_or_prompt,
        "run step 2 --strict"
    );

    assert_eq!(outcome.task.subtasks[2].id, "ST-3");
    assert_eq!(outcome.task.subtasks[2].status, TaskStatus::Queued);
    assert_eq!(outcome.task.subtasks[2].output_excerpt, None);
    assert_eq!(outcome.task.subtasks[2].retry_count, 0);

    // Verify checkpoint metadata in store
    let store = SqliteTaskStore::new(&ws).expect("open store");
    let stored_cp = store
        .get_task_checkpoint(&seeded.id)
        .await
        .expect("get checkpoint")
        .expect("checkpoint exists");
    assert!(stored_cp.verify_integrity_for_task(&seeded.id).is_ok());
    assert_eq!(stored_cp.metadata.completed_subtasks, 1);
    assert_eq!(
        stored_cp.metadata.last_completed_subtask_id.as_deref(),
        Some("ST-1")
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_replay_cli_json_reports_rewound_task() {
    let ws = unique_test_workspace("replay_cli_json");
    let seeded = seed_three_step_task(&ws).await;
    let task_id = seeded.id.0.clone();

    let output = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "replay",
            task_id.as_str(),
            "--from-step",
            "2",
            "--workspace",
            ws.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("run replay command");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let outcome: ReplayOutcomePayload =
        serde_json::from_slice(&output.stdout).expect("parse replay JSON output");
    assert_eq!(outcome.action, "replay");
    assert_eq!(outcome.source_task_id, seeded.id);
    assert_eq!(outcome.task.id, seeded.id);
    assert_eq!(outcome.task.status, TaskStatus::Queued);
    assert_eq!(outcome.from_step, Some(2));
    assert_eq!(outcome.task.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(outcome.task.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(outcome.task.subtasks[2].status, TaskStatus::Queued);

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_fork_task_from_checkpoint_creates_independent_task_inheriting_state() {
    let ws = unique_test_workspace("fork_from_checkpoint");
    let seeded = seed_three_step_task(&ws).await;
    let source_id_str = seeded.id.0.clone();

    // Fork task starting from step 2:
    // - Must create a brand new Task with a distinct TaskId
    // - Must inherit Step 1 (Completed + output_excerpt), while rewinding Step 2 and Step 3 to Queued
    // - Must preserve priority, assigned_agent, engine_name, and allow overriding goal/title
    // - Source task and its checkpoint in SqliteTaskStore must remain completely untouched
    let fork_args = ForkArgs {
        task_id: Some(source_id_str.clone()),
        flag_id: None,
        from_step: Some(2),
        subtask_id: None,
        title: Some("Forked Migration Branch".to_string()),
        goal: Some("Branch migration with alternative transform".to_string()),
        prompt: Some("run step 2 --alt-branch".to_string()),
        workspace: ws.clone(),
        format: OutputFormat::Json,
    };

    let outcome = execute_replay_or_fork(&fork_args.into_replay_args())
        .await
        .expect("execute fork from step 2");

    assert_eq!(outcome.action, "fork");
    assert_eq!(outcome.source_task_id, seeded.id);
    assert_ne!(
        outcome.task.id, seeded.id,
        "Forked task must have a new independent TaskId"
    );
    assert!(outcome.task.id.0.starts_with("TASK-"));
    assert_eq!(outcome.task.title, "Forked Migration Branch");
    assert_eq!(
        outcome.task.goal_description,
        "Branch migration with alternative transform"
    );
    assert_eq!(outcome.task.priority, Priority::High);
    assert_eq!(outcome.task.assigned_agent, seeded.assigned_agent);
    assert_eq!(outcome.task.engine_name, seeded.engine_name);
    assert_eq!(outcome.task.status, TaskStatus::Queued);
    assert_eq!(outcome.task.failure_count, 0);

    // Check inherited checkpoint state on forked task
    assert_eq!(outcome.task.subtasks.len(), 3);
    assert_eq!(outcome.task.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(
        outcome.task.subtasks[0].output_excerpt.as_deref(),
        Some("schema altered v2")
    );
    assert_eq!(outcome.task.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(outcome.task.subtasks[1].output_excerpt, None);
    assert_eq!(
        outcome.task.subtasks[1].command_or_prompt,
        "run step 2 --alt-branch"
    );
    assert_eq!(outcome.task.subtasks[2].status, TaskStatus::Queued);
    assert_eq!(outcome.task.subtasks[2].output_excerpt, None);
    assert_eq!(outcome.task.subtasks[2].retry_count, 0);

    // Verify both source task and forked task exist independently in SqliteTaskStore
    let store = SqliteTaskStore::new(&ws).expect("open store");
    let original_after_fork = store
        .get_task(&seeded.id)
        .await
        .expect("get original task")
        .expect("original task still exists");
    assert!(
        matches!(original_after_fork.status, TaskStatus::Failed { .. }),
        "Original task status must remain untouched"
    );
    assert_eq!(original_after_fork.failure_count, 2);
    assert_eq!(
        original_after_fork.subtasks[1].status,
        TaskStatus::Completed
    );
    assert!(matches!(
        original_after_fork.subtasks[2].status,
        TaskStatus::Failed { .. }
    ));

    let original_cp = store
        .get_task_checkpoint(&seeded.id)
        .await
        .expect("get original checkpoint")
        .expect("original checkpoint still exists");
    assert_eq!(original_cp.metadata.completed_subtasks, 2);

    // Verify forked task and its checkpoint are persisted and valid in SqliteTaskStore
    let persisted_fork = store
        .get_task(&outcome.task.id)
        .await
        .expect("get forked task")
        .expect("forked task persisted");
    assert_eq!(persisted_fork.id, outcome.task.id);
    assert_eq!(persisted_fork.status, TaskStatus::Queued);
    assert_eq!(persisted_fork.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(persisted_fork.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(persisted_fork.subtasks[2].status, TaskStatus::Queued);

    let forked_cp = store
        .get_task_checkpoint(&outcome.task.id)
        .await
        .expect("get forked checkpoint")
        .expect("forked checkpoint persisted");
    assert!(forked_cp
        .verify_integrity_for_task(&outcome.task.id)
        .is_ok());
    assert_eq!(forked_cp.metadata.task_id, outcome.task.id);
    assert_eq!(forked_cp.metadata.completed_subtasks, 1);
    assert_eq!(
        forked_cp.metadata.last_completed_subtask_id.as_deref(),
        Some("ST-1")
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_handle_fork_converts_args_and_persists_forked_task() {
    let ws = unique_test_workspace("handle_fork");
    let seeded = seed_three_step_task(&ws).await;
    let source_id = seeded.id.0.clone();

    let args = ForkArgs {
        task_id: Some(source_id.clone()),
        flag_id: None,
        from_step: Some(2),
        subtask_id: None,
        title: Some("Fork created through handler".to_string()),
        goal: None,
        prompt: None,
        workspace: ws.clone(),
        format: OutputFormat::Json,
    };

    handle_fork(args).await.expect("handle fork command");

    let store = SqliteTaskStore::new(&ws).expect("open store");
    let tasks = store.list_tasks(100, 0).await.expect("list tasks");
    let forked = tasks
        .iter()
        .find(|task| task.id != seeded.id)
        .expect("handler persists a forked task");
    assert_eq!(forked.title, "Fork created through handler");
    assert_eq!(forked.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(forked.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(forked.subtasks[2].status, TaskStatus::Queued);

    let source = store
        .get_task(&seeded.id)
        .await
        .expect("get source task")
        .expect("source task remains stored");
    assert_eq!(source.status, seeded.status);
    assert_eq!(source.subtasks, seeded.subtasks);

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_handle_replay_persists_rewound_task() {
    let ws = unique_test_workspace("handle_replay");
    let seeded = seed_three_step_task(&ws).await;
    let task_id = seeded.id.0.clone();

    let args = ReplayArgs {
        task_id: Some(task_id),
        flag_id: None,
        list: false,
        from_step: Some(2),
        subtask_id: None,
        fork: false,
        title: None,
        goal: None,
        prompt: None,
        workspace: ws.clone(),
        format: OutputFormat::Json,
    };

    handle(args).await.expect("handle replay command");

    let store = SqliteTaskStore::new(&ws).expect("open store");
    let replayed = store
        .get_task(&seeded.id)
        .await
        .expect("get replayed task")
        .expect("replayed task remains stored");
    assert_eq!(replayed.id, seeded.id);
    assert_eq!(replayed.status, TaskStatus::Queued);
    assert_eq!(replayed.failure_count, 0);
    assert_eq!(replayed.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(replayed.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(replayed.subtasks[2].status, TaskStatus::Queued);

    let checkpoint = store
        .get_task_checkpoint(&seeded.id)
        .await
        .expect("get replayed checkpoint")
        .expect("replayed checkpoint remains stored");
    assert_eq!(checkpoint.metadata.completed_subtasks, 1);
    assert_eq!(
        checkpoint.metadata.last_completed_subtask_id.as_deref(),
        Some("ST-1")
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_handle_lists_checkpoints_without_mutating_task() {
    let ws = unique_test_workspace("handle_replay_list");
    let seeded = seed_three_step_task(&ws).await;
    let args = ReplayArgs {
        task_id: Some(seeded.id.0.clone()),
        flag_id: None,
        list: true,
        from_step: None,
        subtask_id: None,
        fork: false,
        title: None,
        goal: None,
        prompt: None,
        workspace: ws.clone(),
        format: OutputFormat::Json,
    };

    handle(args).await.expect("list replay checkpoints");

    let store = SqliteTaskStore::new(&ws).expect("open store");
    let listed_task = store
        .get_task(&seeded.id)
        .await
        .expect("get listed task")
        .expect("task remains stored");
    assert_eq!(listed_task.status, seeded.status);
    assert_eq!(listed_task.subtasks, seeded.subtasks);

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_agent_ctl_cli_binary_replay_list_and_fork_end_to_end() {
    let ws = unique_test_workspace("cli_bin_replay_fork");
    let seeded = seed_three_step_task(&ws).await;
    let source_id_str = seeded.id.0.clone();

    // 1. `agent-ctl replay <TASK_ID> --list --format json`
    let list_out = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "replay",
            &source_id_str,
            "--list",
            "--workspace",
            ws.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("run agent-ctl replay --list");
    assert!(
        list_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&list_out.stderr)
    );
    let list_parsed: CheckpointListPayload =
        serde_json::from_slice(&list_out.stdout).expect("parse CheckpointListPayload JSON");
    assert_eq!(list_parsed.task_id, seeded.id);
    assert_eq!(list_parsed.metadata.completed_subtasks, 2);
    assert_eq!(list_parsed.subtasks.len(), 3);

    // 2. `agent-ctl fork <TASK_ID> --from-step 2 --format json`
    let fork_out = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "fork",
            &source_id_str,
            "--from-step",
            "2",
            "--workspace",
            ws.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("run agent-ctl fork --from-step 2");
    assert!(
        fork_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&fork_out.stderr)
    );
    let fork_parsed: ReplayOutcomePayload =
        serde_json::from_slice(&fork_out.stdout).expect("parse ReplayOutcomePayload JSON");
    assert_eq!(fork_parsed.action, "fork");
    assert_eq!(fork_parsed.source_task_id, seeded.id);
    assert_ne!(fork_parsed.task.id, seeded.id);
    assert_eq!(fork_parsed.task.status, TaskStatus::Queued);
    assert_eq!(fork_parsed.task.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(fork_parsed.task.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(fork_parsed.task.subtasks[2].status, TaskStatus::Queued);

    // 3. `agent-ctl replay <TASK_ID> --from-step 3 --format json` (in-place replay from step 3)
    let replay_out = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "replay",
            &source_id_str,
            "--from-step",
            "3",
            "--workspace",
            ws.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("run agent-ctl replay --from-step 3");
    assert!(
        replay_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&replay_out.stderr)
    );
    let replay_parsed: ReplayOutcomePayload =
        serde_json::from_slice(&replay_out.stdout).expect("parse ReplayOutcomePayload JSON");
    assert_eq!(replay_parsed.action, "replay");
    assert_eq!(replay_parsed.task.id, seeded.id);
    assert_eq!(replay_parsed.task.status, TaskStatus::Queued);
    assert_eq!(replay_parsed.task.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(replay_parsed.task.subtasks[1].status, TaskStatus::Completed);
    assert_eq!(replay_parsed.task.subtasks[2].status, TaskStatus::Queued);
    assert_eq!(replay_parsed.checkpoint.metadata.completed_subtasks, 2);

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_agent_ctl_replay_table_output_formats_checkpoint_list_and_outcome() {
    let ws = unique_test_workspace("cli_bin_replay_table_output");
    let seeded = seed_three_step_task(&ws).await;
    let source_id = seeded.id.0.clone();

    let list_out = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "replay",
            &source_id,
            "--list",
            "--workspace",
            ws.to_str().unwrap(),
            "--format",
            "table",
        ])
        .output()
        .expect("run agent-ctl replay --list in table format");
    assert!(
        list_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&list_out.stderr)
    );
    let list_stdout = String::from_utf8(list_out.stdout).expect("table output is UTF-8");
    assert!(list_stdout.contains(&format!(
        "CHECKPOINT STEPS FOR TASK: {source_id} (Database Migration Pipeline)"
    )));
    assert!(list_stdout.contains("Completed: 2/3 | Last Completed: Some(\"ST-2\")"));
    let list_step_1 = list_stdout
        .find("[Step 1] ST-1 | Completed | Backup & Schema Alter")
        .unwrap();
    let list_step_2 = list_stdout
        .find("[Step 2] ST-2 | Completed | Transform Records")
        .unwrap();
    let list_step_3 = list_stdout.find("[Step 3] ST-3 | Failed").unwrap();
    assert!(list_step_1 < list_step_2 && list_step_2 < list_step_3);

    let outcome_out = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "fork",
            &source_id,
            "--from-step",
            "2",
            "--workspace",
            ws.to_str().unwrap(),
            "--format",
            "table",
        ])
        .output()
        .expect("run agent-ctl fork in table format");
    assert!(
        outcome_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&outcome_out.stderr)
    );
    let outcome_stdout = String::from_utf8(outcome_out.stdout).expect("table output is UTF-8");
    assert!(outcome_stdout.contains(&format!(
        "ACTION: FORK | SOURCE: {source_id} -> TARGET TASK:"
    )));
    assert!(outcome_stdout.contains("| STATUS: Queued"));
    let outcome_step_1 = outcome_stdout
        .find("[Step 1] ST-1 | Completed | Backup & Schema Alter")
        .unwrap();
    let outcome_step_2 = outcome_stdout
        .find("[Step 2] ST-2 | Queued | Transform Records")
        .unwrap();
    let outcome_step_3 = outcome_stdout
        .find("[Step 3] ST-3 | Queued | Verify Checksums")
        .unwrap();
    assert!(outcome_step_1 < outcome_step_2 && outcome_step_2 < outcome_step_3);

    let _ = std::fs::remove_dir_all(&ws);
}
