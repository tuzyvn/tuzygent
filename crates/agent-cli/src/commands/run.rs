use agent_domain::{AgentId, GuardrailPolicy, Priority, Task, TaskStatus};
use agent_engine::config::EnginesConfig;
use agent_memory::{DurableMemoryStore, PersonaLoader, ScratchpadManager};
use agent_runtime::{AutonomousOodaLoop, InterAgentBus, SandboxConfig, SandboxDriverKind};
use std::io::Write;

#[derive(Debug, Clone)]
pub struct RunArgs {
    pub agent_id: String,
    pub goal: String,
    pub workspace: String,
    pub engine: String,
    pub sandbox: Option<String>,
    pub persona_path: Option<String>,
    pub timeout: Option<u64>,
    pub auto_approve: bool,
}

pub async fn handle(args: RunArgs) -> anyhow::Result<()> {
    let RunArgs {
        agent_id,
        goal,
        workspace,
        engine,
        sandbox,
        persona_path,
        timeout,
        auto_approve,
    } = args;

    println!("🚀 Dispatching goal to Agent [{}]: {}", agent_id, goal);

    let ws_path = std::path::Path::new(&workspace);
    let config = EnginesConfig::load_hierarchical(Some(ws_path));
    let engine_name = if engine.is_empty() {
        &config.defaults.engine
    } else {
        &engine
    };

    let compute_engine = agent_engine::configured_engine(&config, engine_name, None)?;
    println!("⚡ Using engine: {}", compute_engine.engine_kind());

    let guardrail = GuardrailPolicy::default_enterprise();
    let scratchpad = ScratchpadManager::new(&workspace);
    let durable = DurableMemoryStore::new(&workspace);

    let profile = match persona_path {
        Some(path) => Some(PersonaLoader::load_from_dir(path).await?),
        None => None,
    };

    let mut sandbox_cfg = SandboxConfig::from_engines_config(&config)?;
    if let Some(ref driver) = sandbox {
        sandbox_cfg.driver = SandboxDriverKind::parse(driver)?;
    }

    println!("🛡️  Active Sandbox Driver: {}", sandbox_cfg.driver);

    let cli_bus = InterAgentBus::new(200);
    let mut cli_rx = cli_bus.subscribe();

    // Background streaming printer for live terminal updates
    let stream_printer = tokio::spawn(async move {
        while let Ok(evt) = cli_rx.recv().await {
            match evt.event_type.as_str() {
                "task.subtask_executing" => {
                    if let Some(title) = evt.payload.get("title").and_then(|t| t.as_str()) {
                        println!("\n🔹 [Executing Step]: {}", title);
                    }
                }
                "task.stream_delta" => {
                    if let Some(delta) = evt.payload.get("delta").and_then(|d| d.as_str()) {
                        print!("{}", delta);
                        let _ = std::io::stdout().flush();
                    }
                }
                "task.git_diff" => {
                    if let Some(diff) = evt.payload.get("diff").and_then(|d| d.as_str()) {
                        println!("\n📊 [Code Diff Detected]:\n{}", diff);
                    }
                }
                _ => {}
            }
        }
    });

    let timeout_secs = timeout.unwrap_or(config.defaults.timeout_seconds);
    let ooda = AutonomousOodaLoop::new(compute_engine, guardrail, scratchpad, durable, &workspace)
        .with_timeout_seconds(timeout_secs)
        .with_sandbox_config(sandbox_cfg)
        .with_bus(cli_bus)
        .with_auto_approve(auto_approve);
    let ooda = if let Some(profile) = profile {
        ooda.with_persona(profile)
    } else {
        ooda
    };

    let mut task = Task::new(&goal, &goal, Priority::High);
    task.assigned_agent = Some(AgentId::from_str(&agent_id));

    let task_result = ooda
        .run_goal_detailed(&mut task)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    stream_printer.abort();
    println!(
        "\n🏁 Mission Finished with Status: {:?}",
        task_result.status
    );
    println!(
        "⏱️  Duration: {}ms | Engine: {}",
        task_result.duration_ms,
        task_result.engine_used.as_deref().unwrap_or("unknown")
    );
    if !task_result.verifications.is_empty() {
        println!(
            "\n🔍 Verification Report ({} checks):",
            task_result.verifications.len()
        );
        for (idx, v) in task_result.verifications.iter().enumerate() {
            let mark = if v.passed { "✅ PASS" } else { "❌ FAIL" };
            println!("  [#{} {}] {}", idx + 1, mark, v.details);
        }
    }
    if task_result.status != TaskStatus::Completed {
        anyhow::bail!("Mission did not complete: {:?}", task_result.status);
    }

    Ok(())
}
