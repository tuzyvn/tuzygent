use std::path::Path;
use agent_domain::{Task, TaskId, TaskResult};
use agent_memory::MemoryService;
use agent_runtime::{SqliteTaskStore, TaskStore, WorkflowDefinition, WorkflowRunReport};
use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::{InspectArgs, InspectTarget, OutputFormat};

#[derive(Serialize)]
struct TaskInspectPayload<'a> {
    task: &'a Task,
    result: Option<&'a TaskResult>,
}

pub async fn handle(args: InspectArgs) -> Result<()> {
    match args.target {
        InspectTarget::Task {
            id,
            flag_id,
            status,
            limit,
        } => handle_task(args.workspace.as_path(), id.or(flag_id), status, limit, args.format).await,
        InspectTarget::Memory {
            id_or_query,
            project,
            persona,
        } => handle_memory(args.workspace.as_path(), id_or_query, &project, &persona, args.format).await,
        InspectTarget::Dag { id, catalog_path } => {
            handle_dag(args.workspace.as_path(), id, &catalog_path, args.format).await
        }
    }
}

async fn handle_task(
    workspace: &Path,
    task_id: Option<String>,
    status_filter: Option<String>,
    limit: usize,
    format: OutputFormat,
) -> Result<()> {
    let store = SqliteTaskStore::new(workspace)
        .context("Failed to open SQLite task store in workspace")?;

    if let Some(id) = task_id {
        let task_id_obj = TaskId(id.clone());
        let (task, result) = store
            .get_task_and_result(&task_id_obj)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found with ID: {}", id))?;

        match format {
            OutputFormat::Json => {
                let payload = TaskInspectPayload {
                    task: &task,
                    result: result.as_ref(),
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            }
            OutputFormat::Table => {
                println!("┌────────────────────────────────────────────────────────────────────────┐");
                println!("│ TASK INSPECTION: {:<53} │", task.id.0);
                println!("├─────────────────┬──────────────────────────────────────────────────────┤");
                println!("│ Title           │ {:<52} │", truncate_str(&task.title, 52));
                println!("│ Status          │ {:<52} │", format!("{:?}", task.status));
                println!("│ Priority        │ {:<52} │", format!("{:?}", task.priority));
                println!(
                    "│ Agent           │ {:<52} │",
                    task.assigned_agent.as_ref().map(|a| a.0.as_str()).unwrap_or("none")
                );
                println!("│ Created At      │ {:<52} │", task.created_at.to_rfc3339());
                println!("│ Subtasks Count  │ {:<52} │", task.subtasks.len());
                println!("└─────────────────┴──────────────────────────────────────────────────────┘");

                if !task.subtasks.is_empty() {
                    println!("\n▶ SUBTASKS ({})", task.subtasks.len());
                    println!("┌──────┬──────────────┬──────────────────┬───────────────────────────────┐");
                    println!("│ #    │ ID           │ Status           │ Title                         │");
                    println!("├──────┼──────────────┼──────────────────┼───────────────────────────────┤");
                    for (idx, sub) in task.subtasks.iter().enumerate() {
                        println!(
                            "│ {:<4} │ {:<12} │ {:<16} │ {:<29} │",
                            idx + 1,
                            truncate_str(&sub.id, 12),
                            truncate_str(&format!("{:?}", sub.status), 16),
                            truncate_str(&sub.title, 29)
                        );
                    }
                    println!("└──────┴──────────────┴──────────────────┴───────────────────────────────┘");
                }

                if let Some(res) = result {
                    println!("\n▶ EXECUTION RESULT");
                    println!("┌─────────────────┬──────────────────────────────────────────────────────┐");
                    println!("│ Result Status   │ {:<52} │", format!("{:?}", res.status));
                    println!("│ Duration (ms)   │ {:<52} │", res.duration_ms);
                    if let Some(ref engine) = res.engine_used {
                        println!("│ Engine          │ {:<52} │", truncate_str(engine, 52));
                    }
                    if !res.summary.is_empty() {
                        println!("│ Summary         │ {:<52} │", truncate_str(&res.summary, 52));
                    }
                    println!("└─────────────────┴──────────────────────────────────────────────────────┘");
                }
            }
        }
    } else {
        let tasks = if let Some(ref st) = status_filter {
            store.list_tasks_by_status(st, limit, 0)?
        } else {
            store.list_tasks(limit, 0).await?
        };

        match format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            }
            OutputFormat::Table => {
                println!("┌──────────────────────────────────────────────────────────────────────────────────────────┐");
                println!("│ REGISTERED TASKS ({:<4})                                                                   │", tasks.len());
                println!("├──────────────────────┬─────────────┬──────────────┬──────────────────────────────────────┤");
                println!("│ Task ID              │ Priority    │ Status       │ Title                                │");
                println!("├──────────────────────┼─────────────┼──────────────┼──────────────────────────────────────┤");
                if tasks.is_empty() {
                    println!("│ (no tasks found)     │             │              │                                      │");
                } else {
                    for t in &tasks {
                        println!(
                            "│ {:<20} │ {:<11} │ {:<12} │ {:<36} │",
                            truncate_str(&t.id.0, 20),
                            truncate_str(&format!("{:?}", t.priority), 11),
                            truncate_str(&format!("{:?}", t.status), 12),
                            truncate_str(&t.title, 36)
                        );
                    }
                }
                println!("└──────────────────────┴─────────────┴──────────────┴──────────────────────────────────────┘");
            }
        }
    }

    Ok(())
}

async fn handle_memory(
    workspace: &Path,
    id_or_query: Option<String>,
    project: &str,
    persona: &str,
    format: OutputFormat,
) -> Result<()> {
    let service = MemoryService::open(workspace, project, persona).ok();

    if let Some(service) = service {
        if let Some(ref target) = id_or_query {
            // First check if it's an exact record ID
            if let Ok(record) = service.show(target) {
                match format {
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&record)?);
                    }
                    OutputFormat::Table => {
                        print_memory_record_table(&record);
                    }
                }
                return Ok(());
            }

            // Otherwise, perform retrieval query
            let retrieved = service.retrieve("cli-inspect", target)?;
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&retrieved)?);
                }
                OutputFormat::Table => {
                    println!("┌────────────────────────────────────────────────────────────────────────┐");
                    println!("│ MEMORY RETRIEVAL: {:<52} │", truncate_str(target, 52));
                    println!("├────────────────────────────────────────────────────────────────────────┤");
                    println!("│ Matched IDs: {:<57} │", retrieved.memory_ids.join(", "));
                    println!("├────────────────────────────────────────────────────────────────────────┤");
                    println!("│ TEXT CONTENT                                                           │");
                    for line in retrieved.text.lines() {
                        println!("│ {:<70} │", truncate_str(line, 70));
                    }
                    println!("└────────────────────────────────────────────────────────────────────────┘");
                }
            }
        } else {
            // List all memory records
            let records = service.list()?;
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&records)?);
                }
                OutputFormat::Table => {
                    print_memory_list_table(&records, persona);
                }
            }
        }
    } else {
        match format {
            OutputFormat::Json => {
                println!("[]");
            }
            OutputFormat::Table => {
                println!("(No memory records found for project='{}', persona='{}')", project, persona);
            }
        }
    }

    Ok(())
}

fn print_memory_record_table(record: &agent_memory::MemoryRecord) {
    println!("┌────────────────────────────────────────────────────────────────────────┐");
    println!("│ MEMORY RECORD: {:<55} │", record.id);
    println!("├─────────────────┬──────────────────────────────────────────────────────┤");
    println!("│ Persona ID      │ {:<52} │", record.persona_id);
    println!("│ Kind            │ {:<52} │", record.kind);
    println!("│ Status          │ {:<52} │", record.status);
    println!("│ Scope           │ {:<52} │", record.scope);
    println!("│ Verified        │ {:<52} │", record.verified);
    println!("│ Conditions      │ {:<52} │", truncate_str(&record.conditions, 52));
    println!("│ Content         │ {:<52} │", truncate_str(&record.content, 52));
    println!("└─────────────────┴──────────────────────────────────────────────────────┘");
}

fn print_memory_list_table(records: &[agent_memory::MemoryRecord], persona: &str) {
    println!("┌────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐");
    println!("│ PERSONA MEMORY ({:<10}) Total: {:<67} │", persona, records.len());
    println!("├──────────────────────────────────────┬─────────────┬───────────┬──────────┬────────────────────────────────────┤");
    println!("│ Record ID                            │ Kind        │ Status    │ Verified │ Content                            │");
    println!("├──────────────────────────────────────┼─────────────┼───────────┼──────────┼────────────────────────────────────┤");
    if records.is_empty() {
        println!("│ (no records found)                   │             │           │          │                                    │");
    } else {
        for r in records {
            println!(
                "│ {:<36} │ {:<11} │ {:<9} │ {:<8} │ {:<34} │",
                truncate_str(&r.id, 36),
                truncate_str(&r.kind, 11),
                truncate_str(&r.status, 9),
                r.verified,
                truncate_str(&r.content, 34)
            );
        }
    }
    println!("└──────────────────────────────────────┴─────────────┴───────────┴──────────┴────────────────────────────────────┘");
}

async fn handle_dag(
    workspace: &Path,
    target_id: Option<String>,
    catalog_path: &str,
    format: OutputFormat,
) -> Result<()> {
    // 1. Attempt to load catalog
    let full_catalog_path = if Path::new(catalog_path).is_absolute() {
        Path::new(catalog_path).to_path_buf()
    } else if workspace.join(catalog_path).exists() {
        workspace.join(catalog_path)
    } else {
        Path::new(catalog_path).to_path_buf()
    };

    let workflows: Vec<WorkflowDefinition> = if full_catalog_path.exists() {
        let content = tokio::fs::read(&full_catalog_path).await?;
        serde_json::from_slice(&content).unwrap_or_default()
    } else {
        Vec::new()
    };

    // 2. Attempt to load execution history
    let history_file = workspace.join("workflows/runs/history.json");
    let run_reports: Vec<WorkflowRunReport> = if history_file.exists() {
        let content = tokio::fs::read(&history_file).await?;
        serde_json::from_slice(&content).unwrap_or_default()
    } else {
        Vec::new()
    };

    if let Some(ref id) = target_id {
        // Find matching workflow
        if let Some(wf) = workflows.iter().find(|w| w.id == *id) {
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(wf)?);
                }
                OutputFormat::Table => {
                    print_workflow_definition_table(wf);
                }
            }
            return Ok(());
        }

        // Find matching run report
        if let Some(report) = run_reports.iter().find(|r| r.run_id == *id || r.workflow_id == *id) {
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(report)?);
                }
                OutputFormat::Table => {
                    print_run_report_table(report);
                }
            }
            return Ok(());
        }

        anyhow::bail!("No workflow definition or run report found matching ID: {}", id);
    } else {
        // List workflows and recent runs
        #[derive(Serialize)]
        struct DagListPayload<'a> {
            workflows: &'a [WorkflowDefinition],
            recent_runs: &'a [WorkflowRunReport],
        }

        match format {
            OutputFormat::Json => {
                let payload = DagListPayload {
                    workflows: &workflows,
                    recent_runs: &run_reports,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            }
            OutputFormat::Table => {
                println!("┌────────────────────────────────────────────────────────────────────────────────────────┐");
                println!("│ WORKFLOW DEFINITIONS ({:<4})                                                            │", workflows.len());
                println!("├──────────────────────┬──────────────────────┬─────────┬───────┬────────────────────────┤");
                println!("│ Workflow ID          │ Name                 │ Version │ Nodes │ Description            │");
                println!("├──────────────────────┼──────────────────────┼─────────┼───────┼────────────────────────┤");
                if workflows.is_empty() {
                    println!("│ (no workflows found) │                      │         │       │                        │");
                } else {
                    for wf in &workflows {
                        println!(
                            "│ {:<20} │ {:<20} │ {:<7} │ {:<5} │ {:<22} │",
                            truncate_str(&wf.id, 20),
                            truncate_str(&wf.name, 20),
                            truncate_str(wf.version.as_deref().unwrap_or("1.0.0"), 7),
                            wf.nodes.len(),
                            truncate_str(wf.description.as_deref().unwrap_or(""), 22)
                        );
                    }
                }
                println!("└──────────────────────┴──────────────────────┴─────────┴───────┴────────────────────────┘");

                if !run_reports.is_empty() {
                    println!("\n┌────────────────────────────────────────────────────────────────────────────────────────┐");
                    println!("│ RECENT RUN REPORTS ({:<4})                                                               │", run_reports.len());
                    println!("├──────────────────────┬──────────────────────┬─────────────┬───────────┬────────────────┤");
                    println!("│ Run ID               │ Workflow ID          │ Status      │ Time (ms) │ Nodes Executed │");
                    println!("├──────────────────────┼──────────────────────┼─────────────┼───────────┼────────────────┤");
                    for r in &run_reports {
                        println!(
                            "│ {:<20} │ {:<20} │ {:<11} │ {:<9} │ {:<14} │",
                            truncate_str(&r.run_id, 20),
                            truncate_str(&r.workflow_id, 20),
                            truncate_str(&r.status, 11),
                            r.duration_ms,
                            r.node_results.len()
                        );
                    }
                    println!("└──────────────────────┴──────────────────────┴─────────────┴───────────┴────────────────┘");
                }
            }
        }
    }

    Ok(())
}

fn print_workflow_definition_table(wf: &WorkflowDefinition) {
    println!("┌────────────────────────────────────────────────────────────────────────┐");
    println!("│ WORKFLOW DAG: {:<56} │", wf.id);
    println!("├─────────────────┬──────────────────────────────────────────────────────┤");
    println!("│ Name            │ {:<52} │", truncate_str(&wf.name, 52));
    println!("│ Version         │ {:<52} │", wf.version.as_deref().unwrap_or("1.0.0"));
    println!(
        "│ Description     │ {:<52} │",
        truncate_str(wf.description.as_deref().unwrap_or(""), 52)
    );
    println!("│ Total Nodes     │ {:<52} │", wf.nodes.len());
    println!("│ Total Edges     │ {:<52} │", wf.edges.len());
    println!("└─────────────────┴──────────────────────────────────────────────────────┘");

    if !wf.nodes.is_empty() {
        println!("\n▶ WORKFLOW NODES");
        println!("┌──────────────┬────────────────────────┬─────────────┬──────────────────────────┐");
        println!("│ Node ID      │ Label / Persona        │ Type        │ Engine / Command         │");
        println!("├──────────────┼────────────────────────┼─────────────┼──────────────────────────┤");
        for n in &wf.nodes {
            let label = n.data.label.as_deref()
                .or(n.data.persona_name.as_deref())
                .unwrap_or("-");
            let extra = n.data.engine.as_deref()
                .or(n.data.command.as_deref())
                .unwrap_or("-");
            println!(
                "│ {:<12} │ {:<22} │ {:<11} │ {:<24} │",
                truncate_str(&n.id, 12),
                truncate_str(label, 22),
                truncate_str(&n.node_type, 11),
                truncate_str(extra, 24)
            );
        }
        println!("└──────────────┴────────────────────────┴─────────────┴──────────────────────────┘");
    }

    if !wf.edges.is_empty() {
        println!("\n▶ WORKFLOW EDGES");
        println!("┌─────────────────────────────┬─────────────────────────────┬──────────────────────┐");
        println!("│ Source Node                 │ Target Node                 │ Label                │");
        println!("├─────────────────────────────┼─────────────────────────────┼──────────────────────┤");
        for e in &wf.edges {
            println!(
                "│ {:<27} │ {:<27} │ {:<20} │",
                truncate_str(&e.source, 27),
                truncate_str(&e.target, 27),
                truncate_str(e.label.as_deref().unwrap_or("-"), 20)
            );
        }
        println!("└─────────────────────────────┴─────────────────────────────┴──────────────────────┘");
    }
}

fn print_run_report_table(report: &WorkflowRunReport) {
    println!("┌────────────────────────────────────────────────────────────────────────┐");
    println!("│ WORKFLOW RUN REPORT: {:<49} │", report.run_id);
    println!("├─────────────────┬──────────────────────────────────────────────────────┤");
    println!("│ Workflow ID     │ {:<52} │", report.workflow_id);
    println!("│ Workflow Name   │ {:<52} │", truncate_str(&report.workflow_name, 52));
    println!("│ Status          │ {:<52} │", report.status);
    println!("│ Duration (ms)   │ {:<52} │", report.duration_ms);
    println!("│ Triggered By    │ {:<52} │", report.triggered_by);
    println!("│ Completed At    │ {:<52} │", report.completed_at);
    println!("└─────────────────┴──────────────────────────────────────────────────────┘");

    if !report.node_results.is_empty() {
        println!("\n▶ EXECUTED NODES ({})", report.node_results.len());
        println!("┌──────────────┬──────────────────┬─────────────┬───────────┬────────────────────┐");
        println!("│ Node ID      │ Name             │ Status      │ Time (ms) │ Engine             │");
        println!("├──────────────┼──────────────────┼─────────────┼───────────┼────────────────────┤");
        for res in report.node_results.values() {
            println!(
                "│ {:<12} │ {:<16} │ {:<11} │ {:<9} │ {:<18} │",
                truncate_str(&res.node_id, 12),
                truncate_str(&res.node_name, 16),
                truncate_str(&res.status, 11),
                res.execution_ms,
                truncate_str(res.engine_used.as_deref().unwrap_or("none"), 18)
            );
        }
        println!("└──────────────┴──────────────────┴─────────────┴───────────┴────────────────────┘");
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        truncated.push_str("...");
        truncated
    }
}
