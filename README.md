# AGENT-CORE: ENTERPRISE AUTONOMOUS MULTI-AGENT PLATFORM IN RUST

Hệ thống Nền tảng Đa Nhân sự Số Tự chủ (**Generic Autonomous Multi-Agent Platform**) độc lập được lập trình 100% bằng ngôn ngữ **Rust**, thiết kế theo mô hình **Clean Architecture & Micro-kernel**.

Dự án hỗ trợ 3 hình thức vận hành linh hoạt:
1. **Mô hình C (Crate SDK Library)**: Nhúng trực tiếp vào bộ nhớ của các ứng dụng Rust khác (Zero overhead, in-process execution).
2. **Mô hình A (CLI Tool - `agent-ctl`)**: Binary dòng lệnh độc lập duy nhất dùng cho quản trị, CI/CD, crontab.
3. **Mô hình B (Daemon Service - `agent-server`)**: Dịch vụ nền REST & WebSocket thời gian thực cho Next.js Dashboard, Mobile App, hoặc hệ thống bên thứ ba.

---

## 1. CẤU TRÚC WORKSPACE (7 CRATES ĐỘC LẬP)

```text
AgentCore/
├── Cargo.toml                  # Workspace Root
├── crates/
│   ├── agent-domain/           # Thực thể: Identity, Persona, Mission, Task, Guardrails, CloudEvents
│   ├── agent-engine/           # Compute Drivers: Antigravity CLI, Claude CLI, Local LLM/OpenAI, Tmux
│   ├── agent-memory/           # Bộ nhớ: Scratchpad (ngắn hạn), Durable Memory (dài hạn), Persona Loader
│   ├── agent-runtime/          # Trái tim vận hành: OODA Loop, InterAgentBus, Supervisor, DAG Orchestrator
│   ├── agent-core/             # SDK Facade & Prelude (Dành riêng cho Mô hình C nhúng trực tiếp)
│   ├── agent-cli/              # Binary thực thi CLI: `agent-ctl`
│   └── agent-server/           # Binary Daemon Microservice: `agent-server` (REST & WebSocket)
└── examples/
    └── embed_agent.rs          # Minh họa nhúng SDK trực tiếp trong 20 dòng code
```

---

## 2. HƯỚNG DẪN SỬ DỤNG MÔ HÌNH C (NHÚNG TRỰC TIẾP LÀM SDK)

### Bước 1: Khai báo dependency vào dự án Rust của bạn
Trong `Cargo.toml` của dự án bên ngoài:

```toml
[dependencies]
agent-core = { path = "../AgentCore/crates/agent-core" }
# hoặc qua git:
# agent-core = { git = "https://github.com/your-org/AgentCore", branch = "main" }
tokio = { version = "1", features = ["full"] }
```

### Bước 2: Nhúng và thực thi in-process

```rust
use agent_core::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Tạo Event Bus và Supervisor trực tiếp trong RAM
    let bus = InterAgentBus::new(100);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));

    // 2. Định nghĩa Identity
    let identity = AgentIdentity {
        id: AgentId::from_str("agent-secops"),
        name: "SecOps Sentinel".to_string(),
        description: "Autonomous Security Auditor".to_string(),
        autonomy_level: AutonomyLevel::Full,
        created_at: chrono::Utc::now(),
    };
    supervisor.register_agent(identity.clone()).await;

    // 3. Khởi tạo OODA Loop
    let engine = Arc::new(AntigravityEngine::new(None, None));
    let scratchpad = ScratchpadManager::new(".");
    let durable = DurableMemoryStore::new(".");
    let guardrails = GuardrailPolicy::default_enterprise();

    let ooda = AutonomousOodaLoop::new(engine, guardrails, scratchpad, durable, ".");

    // 4. Giao nhiệm vụ cho Agent
    let mut task = Task::new("Audit Server", "Kiểm tra bảo mật máy chủ", Priority::High);
    task.assigned_agent = Some(identity.id);

    let status = ooda.run_goal(&mut task).await?;
    println!("Trạng thái hoàn thành: {:?}", status);

    Ok(())
}
```

### Chạy thử ví dụ mẫu:
```bash
cargo run -p agent-core --example embed_agent
```

---

## 3. HƯỚNG DẪN CHẠY DAEMON REST & WEBSOCKET (`agent-server`)

Khởi động server API:
```bash
cargo run -p agent-server -- --host 0.0.0.0 --port 8080 --workspace .
```

### Các Endpoint chính:
- **`GET /health`**: Kiểm tra trạng thái server.
- **`GET /api/v1/engines`**: Xem cấu hình các AI CLI engine đã nạp và trạng thái sandbox OS.
- **`GET /api/v1/agents`**: Lấy danh sách nhân viên số đang hoạt động.
- **`POST /api/v1/agents`**: Đăng ký agent mới.
- **`POST /api/v1/tasks/run`**: Giao mục tiêu cho Agent thực thi tự chủ OODA loop (chọn engine & sandbox).
- **`GET /api/v1/ws`**: Kênh WebSocket stream thời gian thực toàn bộ sự kiện từ `InterAgentBus`.

---

## 4. HƯỚNG DẪN SỬ DỤNG DÒNG LỆNH (`agent-ctl`)

Build binary độc lập:
```bash
cargo build --release -p agent-cli
# Binary nằm tại: target/release/agent-ctl
```

Các lệnh quản trị và điều phối OS AI:
```bash
# 1. Liệt kê và kiểm tra kết nối các AI CLI engine của hệ thống
./target/release/agent-ctl engines --workspace .
# Test nhanh kết nối engine:
./target/release/agent-ctl engines --test fast_echo

# 2. Liệt kê danh sách agent
./target/release/agent-ctl list

# 3. Tạo agent mới
./target/release/agent-ctl create --id "sre-01" --name "SRE DevOps Lead" --autonomy "full"

# 4. Giao task cho Agent (kèm chỉ định engine và sandbox)
./target/release/agent-ctl run --agent-id "sre-01" --goal "Tối ưu hóa Docker và kiểm tra bảo mật" --engine antigravity --sandbox bubblewrap

# 5. Chạy mô phỏng phối hợp đa tác tử (Multi-agent collaboration qua Bus)
./target/release/agent-ctl demo-multi-agent

# 6. Khởi động team đa tác tử theo OpenRig Spec
./target/release/agent-ctl rig up --spec rigs/software-dev.yaml
```

---

## 5. CẤU HÌNH AI CLI ENGINE & SANDBOX (`tuzygent.toml`)

Hệ thống hỗ trợ nạp cấu hình phân tầng từ `~/.tuzygent/engines.toml` (OS-wide) và `tuzygent.toml` (workspace local):

```toml
[defaults]
engine = "antigravity"
timeout_seconds = 180
sandbox_driver = "bubblewrap" # "bubblewrap" (Linux native), "docker", "host"

[security]
allowed_env_vars = ["ANTHROPIC_API_KEY", "GEMINI_API_KEY", "OPENAI_API_KEY", "ANTIGRAVITY_API_KEY"]
mount_credentials = [
    { source = "~/.gemini", target = "/root/.gemini", readonly = true },
    { source = "~/.claude", target = "/root/.claude", readonly = true }
]

[engines.antigravity]
kind = "antigravity"
binary_path = "agy"
args = ["--interactive=false", "--output-format", "stream-json", "--message", "{prompt}"]

[engines.claude]
kind = "claude_code"
binary_path = "claude"
args = ["-p", "{prompt}", "--output-format", "json"]

[engines.generic_ollama]
kind = "generic"
binary_path = "ollama"
args = ["run", "qwen2.5-coder:7b", "{prompt}"]
```

---

## 5. DOCKER & DEPLOYMENT ĐỘC LẬP

```bash
docker build -t agent-core:latest .
docker run -d -p 8080:8080 --name agent-core-daemon agent-core:latest
```
