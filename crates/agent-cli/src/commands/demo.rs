use agent_domain::{
    AgentId, AgentIdentity, AutonomyLevel, GuardrailPolicy, InterAgentEvent, Priority, Task,
    TaskStatus,
};
use agent_engine::api::OpenAiCompatibleEngine;
use agent_engine::cli::AntigravityEngine;
use agent_engine::config::EnginesConfig;
use agent_engine::{ComputeEngine, EngineInput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{
    AgentSupervisor, AutonomousOodaLoop, InterAgentBus, SandboxConfig,
};
use std::path::Path;
use std::sync::Arc;

pub async fn multi_agent(engine: String, workspace: String) -> anyhow::Result<()> {
    println!("🤖🤖 Khởi chạy hệ thống Hợp tác Đa tác tử Tự trị (Real Multi-Agent Collaboration)...");

    let config = EnginesConfig::load_hierarchical(Some(Path::new(&workspace)));
    let engine_name = if engine.is_empty() {
        &config.defaults.engine
    } else {
        &engine
    };
    let compute_engine = agent_engine::configured_engine(&config, engine_name, None)?;
    let sandbox_cfg = SandboxConfig::from_engines_config(&config)?;
    println!("⚡ Using engine: {}", compute_engine.engine_kind());

    let bus = InterAgentBus::new(100);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));

    // 1. Create Agent 1 (DevOps/SRE) and Agent 2 (Core Developer)
    let agent1 = AgentIdentity {
        id: AgentId::from_str("agent-sre-alex"),
        name: "Alex - SRE Incident Commander".to_string(),
        description: "Giám sát hệ thống, phân tích sự cố và điều phối giải pháp".to_string(),
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
        temperature: None,
        max_tokens: None,
        max_output_bytes: None,
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

    let event = InterAgentEvent::new(
        agent1.id.clone(),
        Some(agent2.id.clone()),
        "agent.task.delegated",
        handoff_payload.clone(),
    );

    bus.publish(event).map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "   ✓ [Agent 1 -> Bus] Đã phát sự kiện CloudEvent ủy quyền cho Agent 2:\n     \"{}\"",
        delegation_brief
    );

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

        println!("⚙️ [Agent 2 - Brian] Tự động khởi chạy OODA Loop với ComputeEngine thực...");
        let ooda = AutonomousOodaLoop::new(
            compute_engine.clone(),
            GuardrailPolicy::default_enterprise(),
            ScratchpadManager::new(&workspace),
            DurableMemoryStore::new(&workspace),
            &workspace,
        )
        .with_timeout_seconds(config.defaults.timeout_seconds)
        .with_sandbox_config(sandbox_cfg);

        let mut task = Task::new("Khắc phục sự cố Nginx 502", instructions, Priority::Critical);
        task.assigned_agent = Some(agent2.id.clone());

        let result = ooda.run_goal(&mut task).await.map_err(|e| anyhow::anyhow!(e))?;
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
        let resolution_event = InterAgentEvent::new(
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
        println!(
            "   ✓ [Agent 2 -> Bus] Đã gửi báo cáo nghiệm thu 'agent.task.resolved' về cho Agent 1."
        );
    }

    println!("\n🎉 Quy trình Hợp tác Đa tác tử Tự trị hoàn tất 100% với ComputeEngine thực!");
    Ok(())
}

pub async fn comedy_show(engine: String) -> anyhow::Result<()> {
    println!("🎭 ==================================================================== 🎭");
    println!("   LIVE COMEDY SHOW: 2 AGENT TỰ NGHĨ THOẠI & ĐỐI ĐÁP QUA LÕI THẬT");
    println!("   Diễn viên 1: Agent DevOps Tèo (agent-devops-teo)");
    println!("   Diễn viên 2: Agent Backend Coder Tí (agent-backend-ti)");
    println!("   Engine lựa chọn: {}", engine.to_uppercase());
    println!("🎭 ==================================================================== 🎭\n");

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
                temperature: None,
                max_tokens: None,
                max_output_bytes: None,
            };

            match compute_engine.execute(&input).await {
                Ok(output) => {
                    let clean_text = output.stdout_content.trim().trim_matches('"').to_string();
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
                temperature: None,
                max_tokens: None,
                max_output_bytes: None,
            };

            match compute_engine.execute(&input).await {
                Ok(output) => {
                    let clean_text = output.stdout_content.trim().trim_matches('"').to_string();
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
    Ok(())
}
