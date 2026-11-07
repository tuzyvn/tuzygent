use agent_cli::cli::{Cli, InspectArgs, InspectTarget, OutputFormat};
use clap::Parser;
use std::process::Command;

#[test]
fn test_inspect_parsing_task_and_task_id_alias() {
    // 1. Test parsing "inspect task <id>"
    let cli =
        Cli::try_parse_from(["agent-ctl", "inspect", "task", "task-p0-123"]).expect("parse failed");
    match cli.command {
        agent_cli::Commands::Inspect(InspectArgs {
            format,
            workspace,
            target: InspectTarget::Task { id, flag_id, .. },
        }) => {
            assert_eq!(format, OutputFormat::Table);
            assert_eq!(workspace.to_str().unwrap(), ".");
            assert_eq!(id.as_deref().or(flag_id.as_deref()), Some("task-p0-123"));
        }
        _ => panic!("Expected Inspect command with Task target"),
    }

    // 2. Test parsing alias "inspect task-id <id> --format json"
    let cli = Cli::try_parse_from([
        "agent-ctl",
        "inspect",
        "task-id",
        "task-p0-456",
        "--format",
        "json",
    ])
    .expect("parse failed");
    match cli.command {
        agent_cli::Commands::Inspect(InspectArgs {
            format,
            target: InspectTarget::Task { id, flag_id, .. },
            ..
        }) => {
            assert_eq!(format, OutputFormat::Json);
            assert_eq!(id.as_deref().or(flag_id.as_deref()), Some("task-p0-456"));
        }
        _ => panic!("Expected Inspect command with Task target"),
    }

    // 3. Test parsing with global flags before subcommand
    let cli = Cli::try_parse_from([
        "agent-ctl",
        "inspect",
        "--format",
        "json",
        "task",
        "--id",
        "task-p0-789",
    ])
    .expect("parse failed");
    match cli.command {
        agent_cli::Commands::Inspect(InspectArgs {
            format,
            target: InspectTarget::Task { id, flag_id, .. },
            ..
        }) => {
            assert_eq!(format, OutputFormat::Json);
            assert_eq!(id.as_deref().or(flag_id.as_deref()), Some("task-p0-789"));
        }
        _ => panic!("Expected Inspect command with Task target"),
    }
}

#[test]
fn test_inspect_parsing_memory() {
    // 1. Table format default
    let cli = Cli::try_parse_from(["agent-ctl", "inspect", "memory"]).expect("parse failed");
    match cli.command {
        agent_cli::Commands::Inspect(InspectArgs {
            format,
            target: InspectTarget::Memory { id_or_query, .. },
            ..
        }) => {
            assert_eq!(format, OutputFormat::Table);
            assert_eq!(id_or_query, None);
        }
        _ => panic!("Expected Inspect command with Memory target"),
    }

    // 2. With query and JSON format
    let cli = Cli::try_parse_from([
        "agent-ctl",
        "inspect",
        "memory",
        "error_handling",
        "--format",
        "json",
        "--project",
        "tuzygent",
        "--persona",
        "devops",
    ])
    .expect("parse failed");
    match cli.command {
        agent_cli::Commands::Inspect(InspectArgs {
            format,
            target:
                InspectTarget::Memory {
                    id_or_query,
                    project,
                    persona,
                },
            ..
        }) => {
            assert_eq!(format, OutputFormat::Json);
            assert_eq!(id_or_query.as_deref(), Some("error_handling"));
            assert_eq!(project, "tuzygent");
            assert_eq!(persona, "devops");
        }
        _ => panic!("Expected Inspect command with Memory target"),
    }
}

#[test]
fn test_inspect_parsing_dag() {
    let cli = Cli::try_parse_from([
        "agent-ctl",
        "inspect",
        "dag",
        "workflow-audit-01",
        "--catalog-path",
        "custom_catalog.json",
        "--format",
        "json",
    ])
    .expect("parse failed");
    match cli.command {
        agent_cli::Commands::Inspect(InspectArgs {
            format,
            target: InspectTarget::Dag { id, catalog_path },
            ..
        }) => {
            assert_eq!(format, OutputFormat::Json);
            assert_eq!(id.as_deref(), Some("workflow-audit-01"));
            assert_eq!(catalog_path, "custom_catalog.json");
        }
        _ => panic!("Expected Inspect command with Dag target"),
    }
}

#[test]
fn test_inspect_execution_task_table_and_json() {
    let temp_dir = std::env::temp_dir().join(format!("inspect-task-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Initialize sqlite store in workspace
    let store = agent_runtime::SqliteTaskStore::new(&temp_dir).unwrap();
    let mut task = agent_domain::Task::new(
        "Self-Healing Audit",
        "Verify task store inspection functionality",
        agent_domain::Priority::High,
    );
    let task_id_str = task.id.0.clone();
    let mut subtask = agent_domain::SubTask::new("sub-1", "Run unit tests", "cargo test", "exit 0");
    subtask.status = agent_domain::TaskStatus::Completed;
    task.subtasks.push(subtask);

    // Save task using blocking or tokio runtime
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        use agent_runtime::TaskStore;
        store.save_task(&task).await.unwrap();
    });

    // 1. Run inspect task table format
    let output = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "inspect",
            "task",
            &task_id_str,
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--format",
            "table",
        ])
        .output()
        .expect("failed to execute agent-ctl");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "STDERR: {stderr}");
    assert!(
        stdout.contains(&task_id_str),
        "STDOUT must contain task ID: {stdout}"
    );
    assert!(
        stdout.contains("Self-Healing Audit"),
        "STDOUT must contain title: {stdout}"
    );
    assert!(
        stdout.contains("Run unit tests"),
        "STDOUT must contain subtask: {stdout}"
    );

    // 2. Run inspect task json format
    let output_json = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "inspect",
            "task-id",
            &task_id_str,
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("failed to execute agent-ctl");

    let stdout_json = String::from_utf8_lossy(&output_json.stdout);
    assert!(output_json.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_json).expect("Output must be valid JSON");
    assert_eq!(parsed["task"]["id"], task_id_str);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_inspect_execution_memory_table_and_json() {
    let temp_dir = std::env::temp_dir().join(format!("inspect-mem-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let memory_service = agent_memory::MemoryService::open(&temp_dir, "default", "coder").unwrap();
    let episode_id = memory_service
        .record_episode("task-1", "Fixed unwrap panic", true)
        .unwrap();
    let lesson_id = memory_service
        .propose_lesson(
            "Always handle None safely with context",
            "when unwrap is found",
            &episode_id,
        )
        .unwrap();

    // 1. Inspect memory in table format
    let output = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "inspect",
            "memory",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--persona",
            "coder",
            "--format",
            "table",
        ])
        .output()
        .expect("failed to execute agent-ctl");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "STDERR: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(&lesson_id) || stdout.contains("Always handle None safely"),
        "{stdout}"
    );

    // 2. Inspect memory in json format
    let output_json = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "inspect",
            "memory",
            &lesson_id,
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--persona",
            "coder",
            "--format",
            "json",
        ])
        .output()
        .expect("failed to execute agent-ctl");

    let stdout_json = String::from_utf8_lossy(&output_json.stdout);
    assert!(output_json.status.success());
    let parsed: serde_json::Value = serde_json::from_str(&stdout_json).expect("Valid JSON");
    assert!(parsed.to_string().contains("Always handle None safely"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_inspect_execution_dag_table_and_json() {
    let temp_dir = std::env::temp_dir().join(format!("inspect-dag-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let catalog_path = temp_dir.join("catalog.json");
    let sample_catalog = serde_json::json!([
        {
            "id": "wf-deploy-pipeline",
            "name": "Deployment Pipeline",
            "description": "Continuous deployment workflow",
            "version": "1.2.0",
            "nodes": [
                {
                    "id": "node-1",
                    "type": "build",
                    "data": {
                        "label": "Build Binary",
                        "engine": "cargo"
                    }
                },
                {
                    "id": "node-2",
                    "type": "test",
                    "data": {
                        "label": "Run Healthcheck",
                        "command": "cargo test"
                    }
                }
            ],
            "edges": [
                {
                    "id": "edge-1",
                    "source": "node-1",
                    "target": "node-2",
                    "label": "on-success"
                }
            ]
        }
    ]);
    std::fs::write(
        &catalog_path,
        serde_json::to_string_pretty(&sample_catalog).unwrap(),
    )
    .unwrap();

    // 1. Inspect dag table format
    let output = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "inspect",
            "dag",
            "wf-deploy-pipeline",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--catalog-path",
            catalog_path.to_str().unwrap(),
            "--format",
            "table",
        ])
        .output()
        .expect("failed to execute agent-ctl");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "STDERR: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Deployment Pipeline"), "{stdout}");
    assert!(
        stdout.contains("node-1") || stdout.contains("Build Binary"),
        "{stdout}"
    );

    // 2. Inspect dag json format
    let output_json = Command::new(env!("CARGO_BIN_EXE_agent-ctl"))
        .args([
            "inspect",
            "dag",
            "wf-deploy-pipeline",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--catalog-path",
            catalog_path.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("failed to execute agent-ctl");

    let stdout_json = String::from_utf8_lossy(&output_json.stdout);
    assert!(output_json.status.success());
    let parsed: serde_json::Value = serde_json::from_str(&stdout_json).expect("Valid JSON");
    assert_eq!(parsed["id"], "wf-deploy-pipeline");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
