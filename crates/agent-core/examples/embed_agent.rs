//! Example: Nhúng trực tiếp Core Agent SDK vào ứng dụng Rust (Mô hình C)
//! Chạy ví dụ: `cargo run -p agent-core --example embed_agent`

use agent_core::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    println!("🦀 [Model C Example] Khởi chạy nhúng trực tiếp AgentCore SDK trong process...");

    // 1. Khởi tạo Bus và Supervisor trực tiếp trong bộ nhớ RAM
    let bus = InterAgentBus::new(50);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));

    // 2. Định nghĩa Identity cho Agent
    let secops_id = AgentId::from_str("secops-sentinel");
    let identity = AgentIdentity {
        id: secops_id.clone(),
        name: "SecOps Sentinel".to_string(),
        description: "Autonomous Security & Vulnerability Auditor".to_string(),
        autonomy_level: AutonomyLevel::Full,
        created_at: chrono::Utc::now(),
    };

    supervisor.register_agent(identity.clone()).await;
    println!("✅ Đã đăng ký Agent: {} [{}]", identity.name, identity.id);

    // 3. Khởi tạo Memory và Guardrails
    let scratchpad = ScratchpadManager::new(".");
    let durable = DurableMemoryStore::new(".");
    let guardrails = GuardrailPolicy::default_enterprise();

    // 4. Chọn Compute Engine (Antigravity Native CLI Driver)
    let engine = Arc::new(AntigravityEngine::new(None, None));

    // 5. Khởi tạo OODA Loop thực thi tự chủ
    let _ooda = AutonomousOodaLoop::new(engine, guardrails, scratchpad, durable, ".");

    println!("🤖 Agent sẵn sàng nhận lệnh. Danh sách agent hiện hành trong Supervisor:");
    for (id, state) in supervisor.list_agents().await {
        println!("   • {} - Trạng thái: {:?}", id.name, state);
    }

    println!("\n🎉 Thành công: AgentCore đã được nhúng và vận hành hoàn toàn trong cùng 1 process (Mô hình C)!");
    Ok(())
}
