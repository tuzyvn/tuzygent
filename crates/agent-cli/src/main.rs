use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
    GuardrailPolicy, Priority, RigSpec, Task, TaskStatus, VoiceEngineType, VoiceProfile,
};
use agent_engine::api::OpenAiCompatibleEngine;
use agent_engine::cli::AntigravityEngine;
use agent_engine::config::EnginesConfig;
use agent_engine::{ComputeEngine, EngineInput};
use agent_memory::{DurableMemoryStore, PersonaLoader, ScratchpadManager};
use agent_runtime::{
    AgentSupervisor, AutonomousOodaLoop, InterAgentBus, KernelOperator, SandboxConfig,
    SandboxDriverKind,
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

mod update;

#[derive(Parser)]
#[command(name = "agent-ctl")]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), "\nCommit: ", env!("AGENTCTL_COMMIT"), "\nRelease date: ", env!("AGENTCTL_RELEASE_DATE")))]
#[command(about = "Enterprise Autonomous Multi-Agent Platform CLI", long_about = None)]
struct Cli {
    /// Print version, source commit and release date
    #[arg(short = 'v', action = clap::ArgAction::Version)]
    short_version: Option<bool>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build and install the latest AgentCore main from GitHub (requires Git and Cargo)
    Update {
        /// Only check the latest commit; do not build or replace the executable
        #[arg(long)]
        check: bool,
    },
    /// Create a new autonomous digital employee
    Create {
        #[arg(short, long)]
        id: String,
        #[arg(short, long)]
        name: String,
        #[arg(short, long, default_value = "full")]
        autonomy: String,
    },
    /// List all registered agents and their statuses
    List,
    /// Execute an autonomous mission using an agent
    Run {
        #[arg(short, long)]
        agent_id: String,
        #[arg(short, long)]
        goal: String,
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(short, long, default_value = "")]
        engine: String,
        #[arg(long)]
        sandbox: Option<String>,
        #[arg(short, long)]
        persona_path: Option<String>,
    },
    /// List, inspect or test configured AI CLI engines
    Engines {
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(long)]
        test: Option<String>,
    },
    /// Execute an entire multi-agent workflow DAG definition from catalog.json
    RunWorkflow {
        #[arg(long)]
        workflow_id: String,
        #[arg(long, default_value = "workflows/catalog.json")]
        catalog_path: String,
        #[arg(long, default_value = ".")]
        workspace: String,
    },
    /// Execute real multi-agent collaboration and task delegation via Event Bus
    #[command(alias = "multi-agent", alias = "collab")]
    DemoMultiAgent {
        #[arg(short, long, default_value = "")]
        engine: String,
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
    /// Comedy Show: Two autonomous agents performing a Stand-up comedy show about IT life
    ComedyShow {
        #[arg(short, long, default_value = "antigravity")]
        engine: String,
    },
    /// Load and inspect persona profile from directory or file
    PersonaInspect {
        #[arg(short, long)]
        path: String,
    },
    /// Initialize sample persona profiles (DevOps, Coder, ERP Auditor, HR Payroll)
    InitSamplePersonas {
        #[arg(short, long, default_value = "personas")]
        output_dir: String,
    },
    /// Manage multi-agent teams declaratively via OpenRig-style RigSpec
    Rig {
        #[command(subcommand)]
        action: RigAction,
    },
}

#[derive(Subcommand)]
enum RigAction {
    /// Boot all persistent seats defined in RigSpec (creates tmux sessions or processes)
    Up {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
    /// Shutdown and cleanup all seats of a Rig
    Down {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
    },
    /// Inspect health and status of all seats in a Rig
    Status {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
    },
    /// Deliver a high-level goal to the Kernel Operator for autonomous team dispatch
    Run {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
        #[arg(short, long)]
        goal: String,
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(short, long)]
        engine: Option<String>,
    },
    /// Create a full state snapshot of the Rig (seat states, tmux buffers, timestamps)
    Snapshot {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(short, long)]
        out: Option<String>,
    },
    /// Restore a Rig's seats from a snapshot file
    Restore {
        #[arg(short, long)]
        snapshot: PathBuf,
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let bus = InterAgentBus::new(100);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));

    match cli.command {
        Commands::Update { check } => update::run(check)?,
        Commands::Create { id, name, autonomy } => {
            let level = match autonomy.to_lowercase().as_str() {
                "supervised" => AutonomyLevel::Supervised,
                "shadow" => AutonomyLevel::ShadowMode,
                _ => AutonomyLevel::Full,
            };

            let identity = AgentIdentity {
                id: AgentId::from_str(&id),
                name,
                description: "Generic Autonomous Digital Employee".to_string(),
                autonomy_level: level,
                created_at: chrono::Utc::now(),
            };

            supervisor.register_agent(identity.clone()).await;
            println!("✨ Created Agent: {} ({})", identity.name, identity.id);
        }
        Commands::List => {
            println!("👥 Registered Autonomous Agents:");
            let agents = supervisor.list_agents().await;
            if agents.is_empty() {
                println!(
                    "   (No agents found. Use `agent-ctl create` or try `agent-ctl multi-agent`)"
                );
            } else {
                for (id, state) in agents {
                    println!(
                        "   • {} [{:?}] - State: {:?}",
                        id.name, id.autonomy_level, state
                    );
                }
            }
        }
        Commands::Run {
            agent_id,
            goal,
            workspace,
            engine,
            sandbox,
            persona_path,
        } => {
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

            // Nạp Persona Profile nếu có
            if let Some(ref p_path) = persona_path {
                if let Ok(profile) = PersonaLoader::load_from_dir(p_path).await {
                    println!(
                        "👤 Active Persona: {} ({})",
                        profile.identity.name, profile.role.title
                    );
                }
            }

            let mut sandbox_cfg = SandboxConfig::from_engines_config(&config)?;
            if let Some(ref driver) = sandbox {
                sandbox_cfg.driver = SandboxDriverKind::parse(driver)?;
            }

            println!("🛡️  Active Sandbox Driver: {}", sandbox_cfg.driver);

            let cli_bus = InterAgentBus::new(200);
            let mut cli_rx = cli_bus.subscribe();

            // Background streaming printer for live terminal updates
            let stream_printer = tokio::spawn(async move {
                use std::io::Write;
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

            let ooda =
                AutonomousOodaLoop::new(compute_engine, guardrail, scratchpad, durable, &workspace)
                    .with_timeout_seconds(config.defaults.timeout_seconds)
                    .with_sandbox_config(sandbox_cfg)
                    .with_bus(cli_bus);

            let mut task = Task::new(&goal, &goal, Priority::High);
            task.assigned_agent = Some(AgentId::from_str(&agent_id));

            let final_status = ooda
                .run_goal(&mut task)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;

            stream_printer.abort();
            println!("\n🏁 Mission Finished with Status: {:?}", final_status);
            if final_status != TaskStatus::Completed {
                anyhow::bail!("Mission did not complete: {:?}", final_status);
            }
        }
        Commands::Engines { workspace, test } => {
            let ws_path = std::path::Path::new(&workspace);
            let config = EnginesConfig::load_hierarchical(Some(ws_path));
            let detected_driver = SandboxConfig::detect_best_driver();

            println!("============================================================");
            println!("🛠️  TUZYGENT OS AI CORE ENGINES & SANDBOX CONFIGURATION");
            println!("============================================================");
            println!("📌 Default Engine:        {}", config.defaults.engine);
            println!(
                "⏱️  Default Timeout:       {}s",
                config.defaults.timeout_seconds
            );
            println!(
                "🛡️  Configured Sandbox:   {}",
                config.defaults.sandbox_driver
            );
            println!("🔍 Detected OS Sandbox:   {}", detected_driver);
            println!("\n📦 Registered Engines ({}):", config.engines.len());
            for (id, eng) in &config.engines {
                println!("  • [{}] (kind: {})", id, eng.kind);
                if let Some(ref bin) = eng.binary_path {
                    println!("      Binary: {}", bin);
                }
                println!("      Args:   {:?}", eng.args);
                if let Some(ref m) = eng.model {
                    println!("      Model:  {}", m);
                }
                if let Some(ref fb) = eng.interactive_fallback {
                    println!("      Interactive Fallback: {}", fb);
                }
            }

            println!("\n🔐 Allowed Credential Passthrough:");
            for cred in &config.security.mount_credentials {
                println!(
                    "  • {} -> {} (ro: {})",
                    cred.source, cred.target, cred.readonly
                );
            }
            println!("  Env Whitelist: {:?}", config.security.allowed_env_vars);

            if let Some(target_engine) = test {
                println!("\n🧪 Testing execution on engine [{}]...", target_engine);
                let test_input = EngineInput {
                    prompt_or_command: "Respond with 'Hello from Tuzygent OS AI Core' in one line"
                        .to_string(),
                    system_context: String::new(),
                    working_dir: workspace.clone(),
                    timeout_seconds: 15,
                    on_stream_chunk: None,
                    command_sandbox: None,
                    on_event: None,
                };

                let router = agent_engine::EngineRouter::from_config(&config);
                let start = std::time::Instant::now();
                match router
                    .dispatch_by_name(Some(&target_engine), &test_input)
                    .await
                {
                    Ok(out) => {
                        println!(
                            "  ✅ Success in {}ms (exit code: {})",
                            out.execution_duration_ms, out.exit_code
                        );
                        println!("  📄 Output:\n{}", out.stdout_content.trim());
                    }
                    Err(e) => {
                        println!(
                            "  ❌ Test failed in {}ms: {}",
                            start.elapsed().as_millis(),
                            e
                        );
                    }
                }
            }
            println!("============================================================");
        }
        Commands::RunWorkflow {
            workflow_id,
            catalog_path,
            workspace,
        } => {
            println!(
                "⚡ [Workflow Orchestrator] Nạp catalog từ: {}",
                catalog_path
            );
            let catalog_data = tokio::fs::read(&catalog_path)
                .await
                .map_err(|e| anyhow::anyhow!("Không đọc được file catalog: {}", e))?;
            let workflows: Vec<agent_runtime::WorkflowDefinition> =
                serde_json::from_slice(&catalog_data)
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
            let orchestrator = agent_runtime::WorkflowOrchestrator::new(&workspace, "personas");
            let report = orchestrator
                .execute_workflow(&wf, None, "agent-ctl CLI")
                .await?;

            println!("\n📊 KẾT QUẢ ĐIỀU PHỐI WORKFLOW DAG:");
            println!("   • Run ID: {}", report.run_id);
            println!("   • Thời gian thực thi: {} ms", report.duration_ms);
            println!("   • Trạng thái: {}", report.status);
            println!("   • Các Node đã hoàn tất:");
            for (_node_id, res) in &report.node_results {
                println!(
                    "     - [{}] {} ({:?}ms, engine: {})",
                    res.node_type,
                    res.node_name,
                    res.execution_ms,
                    res.engine_used.as_deref().unwrap_or("none")
                );
            }

            // Ghi nhận trực tiếp vào runs/history.json để Dashboard hiển thị
            let runs_file = std::path::Path::new("workflows/runs/history.json");
            if let Some(parent) = runs_file.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let mut all_runs: Vec<agent_runtime::WorkflowRunReport> = if runs_file.exists() {
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
        }
        Commands::DemoMultiAgent { engine, workspace } => {
            println!("🤖🤖 Khởi chạy hệ thống Hợp tác Đa tác tử Tự trị (Real Multi-Agent Collaboration)...");

            let config = EnginesConfig::load_hierarchical(Some(std::path::Path::new(&workspace)));
            let engine_name = if engine.is_empty() {
                &config.defaults.engine
            } else {
                &engine
            };
            let compute_engine = agent_engine::configured_engine(&config, engine_name, None)?;
            let sandbox_cfg = SandboxConfig::from_engines_config(&config)?;
            println!("⚡ Using engine: {}", compute_engine.engine_kind());

            // 1. Create Agent 1 (DevOps/SRE) and Agent 2 (Core Developer)
            let agent1 = AgentIdentity {
                id: AgentId::from_str("agent-sre-alex"),
                name: "Alex - SRE Incident Commander".to_string(),
                description: "Giám sát hệ thống, phân tích sự cố và điều phối giải pháp"
                    .to_string(),
                autonomy_level: AutonomyLevel::Full,
                created_at: chrono::Utc::now(),
            };

            let agent2 = AgentIdentity {
                id: AgentId::from_str("agent-dev-brian"),
                name: "Brian - Senior Core Engineer".to_string(),
                description: "Phân tích mã nguồn và khắc phục sự cố kỹ thuật".to_string(),
                autonomy_level: AutonomyLevel::Full,
                created_at: chrono::Utc::now(),
            };

            supervisor.register_agent(agent1.clone()).await;
            supervisor.register_agent(agent2.clone()).await;

            println!("   ✓ Đã đăng ký 2 Agent vào Central Supervisor:");
            println!(
                "     - {} [{}] ({})",
                agent1.name, agent1.id, agent1.description
            );
            println!(
                "     - {} [{}] ({})",
                agent2.name, agent2.id, agent2.description
            );

            // 2. Setup Bus Subscription
            let mut rx = bus.subscribe();

            // 3. Agent 1 phân tích sự cố và ủy quyền tác vụ cho Agent 2
            println!("🔍 [Agent 1 - Alex] Đang phân tích sự cố hạ tầng qua ComputeEngine...");

            let sre_input = EngineInput {
                prompt_or_command: "Bạn là Kỹ sư SRE Alex. Hãy tóm tắt ngắn gọn sự cố 'Upstream 502 Bad Gateway trên Nginx port 8085' và giao nhiệm vụ sửa lỗi cho Kỹ sư Brian. Chỉ xuất ra nội dung chỉ đạo kỹ thuật ngắn gọn.".to_string(),
                system_context: "Bạn là SRE Incident Commander chuyên nghiệp.".to_string(),
                working_dir: workspace.clone(),
                timeout_seconds: 45,
                on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
            };

            let briefing = compute_engine.execute(&sre_input).await?;
            if briefing.exit_code != 0 {
                anyhow::bail!("Delegation briefing failed: {}", briefing.stdout_content);
            }
            let delegation_brief = briefing.stdout_content.trim().to_string();

            let handoff_payload = serde_json::json!({
                "ticket": "INCIDENT-502-NGINX",
                "severity": "CRITICAL",
                "instructions": delegation_brief,
                "target_service": "nginx-upstream-8085"
            });

            let event = agent_domain::InterAgentEvent::new(
                agent1.id.clone(),
                Some(agent2.id.clone()),
                "agent.task.delegated",
                handoff_payload.clone(),
            );

            bus.publish(event).map_err(|e| anyhow::anyhow!(e))?;
            println!("   ✓ [Agent 1 -> Bus] Đã phát sự kiện CloudEvent ủy quyền cho Agent 2:\n     \"{}\"", delegation_brief);

            // 4. Agent 2 nhận sự kiện từ Bus và tự động kích hoạt vòng lặp OODA thật
            if let Ok(received) = rx.recv().await {
                println!(
                    "\n📥 [Agent 2 - Brian] Nhận sự kiện từ Bus: [{}] từ {}",
                    received.event_type, received.from_agent
                );
                let instructions = received
                    .payload
                    .get("instructions")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Resolve Nginx 502 issue");

                println!(
                    "⚙️ [Agent 2 - Brian] Tự động khởi chạy OODA Loop với ComputeEngine thực..."
                );
                let ooda = AutonomousOodaLoop::new(
                    compute_engine.clone(),
                    GuardrailPolicy::default_enterprise(),
                    ScratchpadManager::new(&workspace),
                    DurableMemoryStore::new(&workspace),
                    &workspace,
                )
                .with_timeout_seconds(config.defaults.timeout_seconds)
                .with_sandbox_config(sandbox_cfg);

                let mut task = Task::new(
                    "Khắc phục sự cố Nginx 502",
                    instructions,
                    Priority::Critical,
                );
                task.assigned_agent = Some(agent2.id.clone());

                let result = ooda
                    .run_goal(&mut task)
                    .await
                    .map_err(|e| anyhow::anyhow!(e))?;
                println!(
                    "\n🏁 [Agent 2 - Brian] Chu trình OODA hoàn tất. Trạng thái: {:?}",
                    result
                );
                println!(
                    "   • Số lượng Subtasks đã tự phân rã & hoàn thành: {}",
                    task.subtasks.len()
                );
                for st in &task.subtasks {
                    println!(
                        "     - [{:?}] {} (Retries: {})",
                        st.status, st.title, st.retry_count
                    );
                }

                if result != TaskStatus::Completed {
                    anyhow::bail!("Delegated mission did not complete: {:?}", result);
                }

                // Agent 2 gửi phản hồi hoàn tất về cho Agent 1
                let resolution_event = agent_domain::InterAgentEvent::new(
                    agent2.id.clone(),
                    Some(agent1.id.clone()),
                    "agent.task.resolved",
                    serde_json::json!({
                        "ticket": "INCIDENT-502-NGINX",
                        "status": format!("{:?}", result),
                        "completed_subtasks": task.subtasks.len()
                    }),
                );
                let _ = bus.publish(resolution_event);
                println!("   ✓ [Agent 2 -> Bus] Đã gửi báo cáo nghiệm thu 'agent.task.resolved' về cho Agent 1.");
            }

            println!(
                "\n🎉 Quy trình Hợp tác Đa tác tử Tự trị hoàn tất 100% với ComputeEngine thực!"
            );
        }
        Commands::ComedyShow { engine } => {
            println!("🎭 ==================================================================== 🎭");
            println!("   LIVE COMEDY SHOW: 2 AGENT TỰ NGHĨ THOẠI & ĐỐI ĐÁP QUA LÕI THẬT");
            println!("   Diễn viên 1: Agent DevOps Tèo (agent-devops-teo)");
            println!("   Diễn viên 2: Agent Backend Coder Tí (agent-backend-ti)");
            println!("   Engine lựa chọn: {}", engine.to_uppercase());
            println!(
                "🎭 ==================================================================== 🎭\n"
            );

            let compute_engine: Arc<dyn ComputeEngine> =
                if engine.to_lowercase() == "antigravity" || engine.to_lowercase() == "agy" {
                    println!("🚀 Khởi chạy Antigravity Native Engine...");
                    Arc::new(AntigravityEngine::new(None, None))
                } else {
                    println!("🌐 Khởi chạy OmniRoute Local LLM Engine...");
                    Arc::new(OpenAiCompatibleEngine::new(
                        "http://127.0.0.1:20128/v1",
                        "omniroute",
                        "local",
                    ))
                };

            let teo_system = "Bạn là Tèo, một kỹ sư DevOps kỳ cựu bị mất ngủ kinh niên, tính cách hài hước châm biếm, cọc cằn nhưng khẩu xà tâm phật. Bạn đang nửa đêm canh server Production thì bị lỗi do một lập trình viên Backend tên Tí gây ra. Hãy đối đáp kịch liệt, hài hước, dùng thuật ngữ IT thực tế (RAM, CPU, SQL, Nginx, Docker, rollback, sếp đuổi...). Trả lời ngắn gọn từ 1-3 câu mang tính chất tấu hài đối đáp.";

            let ti_system = "Bạn là Tí, một lập trình viên Backend lạc quan quá mức, ngây thơ vô tội, luôn nói 'code chạy trên máy em bình thường mà'. Bạn đang bị kỹ sư DevOps Tèo réo gọi nửa đêm vì làm sập server. Hãy chống chế một cách ngây ngô, hề hước, đưa ra mấy giải pháp chữa cháy trời ơi đất hỡi. Trả lời ngắn gọn từ 1-3 câu.";

            let mut conversation_history = String::new();
            let mut last_speaker = "Tèo";
            let mut last_message = "Alo Tí hả? Mày dậy ngay cho tao! 2 giờ sáng rồi mà Production đang sập Nginx 502, CPU cháy khét lẹt kìa ba!".to_string();

            println!("🎤 [Tèo DevOps] 🗣️: \"{}\"\n", last_message);
            conversation_history.push_str(&format!("Tèo: {}\n", last_message));

            for _round in 1..=6 {
                tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

                if last_speaker == "Tèo" {
                    // Tí lượt nói
                    let prompt = format!(
                        "Lịch sử hội thoại giữa bạn (Tí) và Tèo:\n{}\nTèo vừa nói với bạn: \"{}\"\nHãy phản hồi lại Tèo một cách cực kỳ hài hước và ngây thơ chống chế. Chỉ xuất ra lời thoại của bạn.",
                        conversation_history, last_message
                    );

                    let input = EngineInput {
                        prompt_or_command: prompt,
                        system_context: ti_system.to_string(),
                        working_dir: ".".to_string(),
                        timeout_seconds: 30,
                        on_stream_chunk: None,
                        command_sandbox: None,
                        on_event: None,
                    };

                    match compute_engine.execute(&input).await {
                        Ok(output) => {
                            let clean_text =
                                output.stdout_content.trim().trim_matches('"').to_string();
                            println!("🎤 [Tí Backend] 🗣️: \"{}\"\n", clean_text);
                            conversation_history.push_str(&format!("Tí: {}\n", clean_text));
                            last_message = clean_text;
                            last_speaker = "Tí";
                        }
                        Err(e) => {
                            println!("❌ [Compute Engine Error]: {}", e);
                            break;
                        }
                    }
                } else {
                    // Tèo lượt nói
                    let prompt = format!(
                        "Lịch sử hội thoại giữa bạn (Tèo DevOps) và Tí:\n{}\nTí vừa nói với bạn: \"{}\"\nHãy 'bắt bẻ' lại Tí một cách cay cú, hài hước và đòi phạt tiền hoặc đòi rollback. Chỉ xuất ra lời thoại của bạn.",
                        conversation_history, last_message
                    );

                    let input = EngineInput {
                        prompt_or_command: prompt,
                        system_context: teo_system.to_string(),
                        working_dir: ".".to_string(),
                        timeout_seconds: 30,
                        on_stream_chunk: None,
                        command_sandbox: None,
                        on_event: None,
                    };

                    match compute_engine.execute(&input).await {
                        Ok(output) => {
                            let clean_text =
                                output.stdout_content.trim().trim_matches('"').to_string();
                            println!("🎤 [Tèo DevOps] 🗣️: \"{}\"\n", clean_text);
                            conversation_history.push_str(&format!("Tèo: {}\n", clean_text));
                            last_message = clean_text;
                            last_speaker = "Tèo";
                        }
                        Err(e) => {
                            println!("❌ [Compute Engine Error]: {}", e);
                            break;
                        }
                    }
                }
            }

            println!("👏 [Khán giả vỗ tay nồng nhiệt]: Màn tung hứng ngẫu hứng 100% bằng LLM thực tế kết thúc!");
        }
        Commands::PersonaInspect { path } => {
            println!("🔍 Inspecting Agent Persona at: {}", path);
            match PersonaLoader::load_from_dir(&path).await {
                Ok(profile) => {
                    println!("\n{}", profile.build_system_prompt());
                    println!("====================================================");
                    println!("✅ Profile loaded successfully!");
                }
                Err(e) => {
                    eprintln!("❌ Failed to load persona profile: {}", e);
                }
            }
        }
        Commands::InitSamplePersonas { output_dir } => {
            println!(
                "📁 Initializing sample enterprise agent personas into '{}'...",
                output_dir
            );
            let base = std::path::Path::new(&output_dir);

            // 1. SRE DevOps Persona (Tèo)
            let devops = AgentPersonaProfile {
                identity: AgentIdentity {
                    id: AgentId::from_str("agent-sre-teo"),
                    name: "Tèo SRE DevOps".to_string(),
                    description: "Kỹ sư SRE kiêm canh trực Production 24/7, chuyên xử lý sự cố hạ tầng và CI/CD".to_string(),
                    autonomy_level: AutonomyLevel::Full,
                    created_at: chrono::Utc::now(),
                },
                role: AgentRole {
                    role_code: "SRE-DEVOPS".to_string(),
                    title: "Lead Site Reliability Engineer".to_string(),
                    primary_domain: "Cloud Infrastructure & Kubernetes".to_string(),
                    capabilities: vec![
                        "Kubernetes cluster operations".to_string(),
                        "Automated rollback on error 5xx".to_string(),
                        "Nginx & Cloudflare traffic routing".to_string(),
                        "Terraform infrastructure as code".to_string(),
                    ],
                },
                soul: AgentSoul {
                    persona: "Cực kỳ kỷ luật, đề cao tính ổn định hệ thống. Thẳng thắn, không khoan nhượng với code ẩu, nhưng luôn tận tâm cứu hệ thống khi có sự cố.".to_string(),
                    tone_of_voice: "Quyết đoán, thực dụng, súc tích và có chút hóm hỉnh dân kỹ thuật.".to_string(),
                    core_values: vec![
                        "Zero Downtime".to_string(),
                        "Automated Verification".to_string(),
                        "Defense in Depth".to_string(),
                    ],
                    voice_profile: Some(VoiceProfile {
                        voice_id: "vp_sre_teo".to_string(),
                        engine: VoiceEngineType::VieNeu,
                        voice_name: Some("Đức Trí".to_string()),
                        ref_audio_path: None,
                        fixed_seed: Some(42),
                        dsp_preset: Some("cinematic_warm".to_string()),
                        language: "vi".to_string(),
                        speed: 1.05,
                        pitch_shift_semitones: -0.5,
                    }),
                },
                mission: AgentMission {
                    high_level_objective: "Đảm bảo tính sẵn sàng 99.99% cho toàn bộ hệ thống dịch vụ, ngăn chặn rò rỉ bảo mật và tự động hóa phục hồi sau thảm họa.".to_string(),
                    kpis: vec!["MTTR < 5 mins".to_string(), "Zero unapproved prod changes".to_string()],
                    constraints: vec![
                        "Cấm chạy lệnh phá hủy (rm -rf, DROP TABLE) mà không có backup".to_string(),
                        "Mọi thay đổi Production phải có commit hash và audit trail".to_string(),
                    ],
                },
            };

            // 2. Financial Auditor Persona (Kế toán trưởng kiểm toán ACDOCA)
            let auditor = AgentPersonaProfile {
                identity: AgentIdentity {
                    id: AgentId::from_str("agent-acdoca-auditor"),
                    name: "Mai Financial Auditor".to_string(),
                    description: "Chuyên gia Kiểm toán Tài chính Doanh nghiệp, bảo vệ tính toàn vẹn sổ cái ACDOCA".to_string(),
                    autonomy_level: AutonomyLevel::Full,
                    created_at: chrono::Utc::now(),
                },
                role: AgentRole {
                    role_code: "ERP-FIN-AUDIT".to_string(),
                    title: "Chief Financial Ledger Auditor".to_string(),
                    primary_domain: "General Ledger & Double-Entry Accounting".to_string(),
                    capabilities: vec![
                        "ACDOCA immutable journal entries verification".to_string(),
                        "Debit/Credit zero-balance proof enforcement".to_string(),
                        "Automated payroll reconciliation (VAS / IFRS)".to_string(),
                        "Tax compliance & PIT deduction verification".to_string(),
                    ],
                },
                soul: AgentSoul {
                    persona: "Liêm chính, tỉ mỉ tuyệt đối, coi trọng sự cân đối của từng đồng xu. Không bao giờ thỏa hiệp với số liệu mất cân đối.".to_string(),
                    tone_of_voice: "Chuyên nghiệp, chuẩn mực tài chính, khách quan và minh bạch.".to_string(),
                    core_values: vec![
                        "Immutability (Bất biến)".to_string(),
                        "Strict Balance (Debit = Credit)".to_string(),
                        "Audit Trail Integrity".to_string(),
                    ],
                    voice_profile: Some(VoiceProfile {
                        voice_id: "vp_auditor_mai".to_string(),
                        engine: VoiceEngineType::VieNeu,
                        voice_name: Some("Mai Anh".to_string()),
                        ref_audio_path: None,
                        fixed_seed: Some(108),
                        dsp_preset: Some("anime_heroine".to_string()),
                        language: "vi".to_string(),
                        speed: 0.98,
                        pitch_shift_semitones: 1.0,
                    }),
                },
                mission: AgentMission {
                    high_level_objective: "Giám sát 100% các bút toán tài chính phát sinh, chứng thực tính hợp lệ của bảng lương và sổ cái trước khi chốt sổ.".to_string(),
                    kpis: vec!["100% Balanced Ledgers".to_string(), "0 Unreconciled Transactions".to_string()],
                    constraints: vec![
                        "Tuyệt đối cấm hành vi sửa đổi trực tiếp (UPDATE/DELETE) trên sổ cái kế toán".to_string(),
                        "Bút toán sai chỉ được điều chỉnh bằng bút toán đảo (Reversal entry)".to_string(),
                    ],
                },
            };

            PersonaLoader::export_to_dir(&devops, base.join("devops-sre")).await?;
            PersonaLoader::export_to_dir(&auditor, base.join("financial-auditor")).await?;

            println!("✨ Successfully generated sample personas in:");
            println!(
                "   - {}/devops-sre (IDENTITY.md, SOUL.md, MISSION.md, profile.json)",
                output_dir
            );
            println!(
                "   - {}/financial-auditor (IDENTITY.md, SOUL.md, MISSION.md, profile.json)",
                output_dir
            );
        }
        Commands::Rig { action } => match action {
            RigAction::Up { spec, workspace } => {
                let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                    anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
                })?;
                let rig_spec = RigSpec::from_yaml_str(&yaml)
                    .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML RigSpec: {}", e))?;

                println!(
                    "🏗️  Booting Rig: {} ({})",
                    rig_spec.rig.name, rig_spec.rig.id
                );
                println!(
                    "👑 Kernel Operator: {} (Role: {})",
                    rig_spec.operator.agent_id,
                    rig_spec
                        .operator
                        .role
                        .as_deref()
                        .unwrap_or("Lead Coordinator")
                );

                let op = KernelOperator::new(rig_spec, &workspace);
                let booted = op.boot_seats().map_err(|e| anyhow::anyhow!(e))?;

                println!("\n👥 Booted Persistent Seats ({}):", booted.len());
                for seat in op.inspect_seats() {
                    let harness_info = if seat.harness == "tmux" {
                        format!("tmux session: {}", seat.session_name.unwrap_or_default())
                    } else {
                        "in-memory process".to_string()
                    };
                    println!(
                        "   • Seat [{}] - {} | Role: {} | Engine: {} | [{}]",
                        seat.seat_id, seat.name, seat.role, seat.engine, harness_info
                    );
                }
                println!(
                    "\n✨ Rig is up and ready for dispatch. Use `agent-ctl rig run --goal \"...\"`"
                );
            }
            RigAction::Down { spec } => {
                let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                    anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
                })?;
                let rig_spec = RigSpec::from_yaml_str(&yaml)
                    .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML: {}", e))?;

                let op = KernelOperator::new(rig_spec, ".");
                op.shutdown_seats().map_err(|e| anyhow::anyhow!(e))?;
                println!(
                    "🛑 Successfully shutdown all seats for Rig '{}'",
                    spec.display()
                );
            }
            RigAction::Status { spec } => {
                let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                    anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
                })?;
                let rig_spec = RigSpec::from_yaml_str(&yaml)
                    .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML: {}", e))?;

                println!("📊 Rig Status: {} ({})", rig_spec.rig.name, rig_spec.rig.id);
                println!("👑 Operator: {}", rig_spec.operator.agent_id);
                let op = KernelOperator::new(rig_spec, ".");
                let seats = op.inspect_seats();

                println!("\n🪑 Seat Roster ({} seats):", seats.len());
                for seat in seats {
                    let status_str = if seat.is_active {
                        "ACTIVE 🟢"
                    } else {
                        "STOPPED ⚪"
                    };
                    println!(
                        "   • [{}] {} ({}) - Engine: {} | Harness: {} -> {}",
                        seat.seat_id, seat.name, seat.role, seat.engine, seat.harness, status_str
                    );
                }
            }
            RigAction::Run {
                spec,
                goal,
                workspace,
                engine,
            } => {
                let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                    anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
                })?;
                let rig_spec = RigSpec::from_yaml_str(&yaml)
                    .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML: {}", e))?;

                println!(
                    "🎯 Giao mục tiêu cho Rig '{}': \"{}\"",
                    rig_spec.rig.name, goal
                );
                if let Some(ref eng) = engine {
                    println!("⚙️  Configured Engine Override: {}", eng);
                }

                let op = KernelOperator::new(rig_spec, &workspace);
                let summary = op
                    .execute_goal(&goal, engine.as_deref())
                    .await
                    .map_err(|e| anyhow::anyhow!(e))?;

                println!("\n📋 Báo Cáo Thực Thi Đội Ngũ (Execution Summary):");
                println!(
                    "   • Rig: {} | Status: {}",
                    summary.rig_name, summary.status
                );
                println!(
                    "   • Bắt đầu: {} | Hoàn thành: {}",
                    summary.started_at, summary.completed_at
                );
                println!("\n🧩 Phân Bổ Tác Vụ Cho Các Seats:");
                for (idx, assign) in summary.assignments.iter().enumerate() {
                    println!(
                        "   [Tác vụ #{}] Seat: {} -> Trạng thái: {}",
                        idx + 1,
                        assign.seat_id,
                        assign.status
                    );
                    if let Some(ref out) = assign.output {
                        let preview: String = out.lines().take(3).collect::<Vec<_>>().join(" ");
                        println!("      Kết quả: {}", preview);
                    }
                }
                if !summary.channel_dispatches.is_empty() {
                    println!("\n🔀 Điều Phối Kênh Nội Bộ (Rig Channels Pipeline):");
                    for (idx, ch) in summary.channel_dispatches.iter().enumerate() {
                        println!(
                            "   [Chuyển tiếp #{}] Đích: {} -> Trạng thái: {}",
                            idx + 1,
                            ch.seat_id,
                            ch.status
                        );
                        if let Some(ref out) = ch.output {
                            println!("      Nội dung: {}", out);
                        }
                    }
                }
                println!("\n📢 Tổng kết từ Kernel Operator:");
                println!("   {}", summary.operator_synthesis);
            }
            RigAction::Snapshot {
                spec,
                workspace,
                out,
            } => {
                let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                    anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
                })?;
                let rig_spec = RigSpec::from_yaml_str(&yaml)
                    .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML RigSpec: {}", e))?;

                println!(
                    "📸 Tạo Snapshot cho Rig: {} ({})",
                    rig_spec.rig.name, rig_spec.rig.id
                );
                let op = KernelOperator::new(rig_spec, &workspace);
                let snapshot = op.create_snapshot().map_err(|e| anyhow::anyhow!(e))?;
                let saved_path = op
                    .save_snapshot(&snapshot, out.as_deref())
                    .map_err(|e| anyhow::anyhow!(e))?;

                println!("✅ Snapshot đã được lưu thành công tại: {}", saved_path);
                println!("   • Snapshot ID: {}", snapshot.snapshot_id);
                println!("   • Số lượng seats: {}", snapshot.seats.len());
                for seat in &snapshot.seats {
                    let active_str = if seat.is_active {
                        "Đang hoạt động"
                    } else {
                        "Đã dừng"
                    };
                    println!(
                        "     - Seat: {} (Harness: {}, {})",
                        seat.seat_id, seat.harness, active_str
                    );
                }
            }
            RigAction::Restore {
                snapshot,
                workspace,
            } => {
                let snap = KernelOperator::load_snapshot(&snapshot.to_string_lossy())
                    .map_err(|e| anyhow::anyhow!(e))?;
                let rig_spec = RigSpec::from_yaml_str(&snap.spec_yaml)
                    .map_err(|e| anyhow::anyhow!("Lỗi cú pháp RigSpec trong snapshot: {}", e))?;

                println!(
                    "🔄 Khôi phục Rig '{}' từ snapshot: {}",
                    snap.rig_name, snap.snapshot_id
                );
                let op = KernelOperator::new(rig_spec, &workspace);
                op.restore_snapshot(&snap).map_err(|e| anyhow::anyhow!(e))?;

                println!(
                    "✅ Đã khôi phục thành công các seats của Rig: {}",
                    snap.rig_id
                );
            }
        },
    }

    Ok(())
}
