use agent_runtime::{WorkflowDefinition, WorkflowOrchestrator, WorkflowRunReport};
use std::path::Path;

pub async fn handle(
    workflow_id: String,
    catalog_path: String,
    workspace: String,
) -> anyhow::Result<()> {
    println!(
        "⚡ [Workflow Orchestrator] Nạp catalog từ: {}",
        catalog_path
    );
    let catalog_data = tokio::fs::read(&catalog_path)
        .await
        .map_err(|e| anyhow::anyhow!("Không đọc được file catalog: {}", e))?;
    let workflows: Vec<WorkflowDefinition> = serde_json::from_slice(&catalog_data)
        .map_err(|e| anyhow::anyhow!("Lỗi parse JSON catalog: {}", e))?;

    let wf = workflows
        .into_iter()
        .find(|w| w.id == workflow_id)
        .ok_or_else(|| anyhow::anyhow!("Không tìm thấy workflow có ID: {}", workflow_id))?;

    println!(
        "🚀 Bắt đầu điều phối DAG: {} (v{})",
        wf.name,
        wf.version.as_deref().unwrap_or("1.0.0")
    );
    let orchestrator = WorkflowOrchestrator::new(&workspace, "personas");
    let report = orchestrator
        .execute_workflow(&wf, None, "agent-ctl CLI")
        .await?;

    println!("\n📊 KẾT QUẢ ĐIỀU PHỐI WORKFLOW DAG:");
    println!("   • Run ID: {}", report.run_id);
    println!("   • Thời gian thực thi: {} ms", report.duration_ms);
    println!("   • Trạng thái: {}", report.status);
    println!("   • Các Node đã hoàn tất:");
    for res in report.node_results.values() {
        println!(
            "     - [{}] {} ({:?}ms, engine: {})",
            res.node_type,
            res.node_name,
            res.execution_ms,
            res.engine_used.as_deref().unwrap_or("none")
        );
    }

    // Ghi nhận trực tiếp vào runs/history.json để Dashboard hiển thị
    let runs_file = Path::new("workflows/runs/history.json");
    if let Some(parent) = runs_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut all_runs: Vec<WorkflowRunReport> = if runs_file.exists() {
        tokio::fs::read(runs_file)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    all_runs.insert(0, report);
    if let Ok(json_bytes) = serde_json::to_vec_pretty(&all_runs) {
        let _ = tokio::fs::write(runs_file, json_bytes).await;
        println!("💾 Đã ghi nhận phiên chạy vào workflows/runs/history.json");
    }

    Ok(())
}
