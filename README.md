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
# Set TUZYGENT_API_TOKEN before starting the daemon.
cargo run -p agent-server -- --host 127.0.0.1 --port 8080 --workspace .
```

### Các Endpoint chính:
- **`GET /health`**: Kiểm tra trạng thái server.
- **`GET /api/v1/engines`**: Xem cấu hình các AI CLI engine đã nạp và trạng thái sandbox OS.
- **`GET /api/v1/agents`**: Lấy danh sách nhân viên số đang hoạt động.
- **`POST /api/v1/agents`**: Đăng ký agent mới.
- **`POST /api/v1/tasks/run`**: Giao mục tiêu cho Agent thực thi tự chủ OODA loop (chọn engine & sandbox).
- **`POST /api/v1/tasks`**: Gửi task chạy nền và nhận ID theo dõi. Có thể gửi header `Idempotency-Key` (1–128 ký tự ASCII hiển thị): gửi lại cùng title, goal, priority và engine trả cùng ID; dùng cùng khóa cho nội dung khác trả HTTP 409. Khóa được lưu trong SQLite và không tự hết hạn.
- **`GET /api/v1/tasks/{id}`**: Xem trạng thái và kết quả task.
- **`POST /api/v1/tasks/{id}/review`**: Duyệt hoặc từ chối bước đã chạy đang chờ xác minh thủ công. Lấy `review_revision` từ GET, xem output và gửi JSON `{"revision":"...","reviewer":"operator","approved":true}`. Phiên bản cũ bị từ chối; bước đã duyệt không chạy lại.
- **`GET /api/v1/ws`**: Kênh WebSocket stream thời gian thực toàn bộ sự kiện từ `InterAgentBus`.

Khi daemon khởi động lại, task còn `Queued` và có engine hợp lệ được đưa lại vào hàng đợi. Checkpoint ghi kế hoạch và trạng thái trước/sau từng bước, giúp xem bước nào đã hoàn thành và bước nào đang thực thi. Task đã bắt đầu chạy được chuyển sang chờ đối soát vì side effect có thể đã xảy ra; hệ thống không tự chạy lại. Chỉ một daemon được sở hữu workspace tại một thời điểm. Task cũ thiếu thông tin engine cần được xử lý thủ công. Review sau crash và phê duyệt hành động bị policy chặn chưa được hỗ trợ. Review kết quả thủ công hiện chỉ hỗ trợ workspace local; output trong task là trích đoạn. API dùng token chung nên `reviewer` là nhãn tự khai, không phải danh tính được xác thực.

---

## 4. HƯỚNG DẪN SỬ DỤNG DÒNG LỆNH (`agent-ctl`)

Build binary độc lập:
```bash
cargo build --release -p agent-cli
# Binary nằm tại: target/release/agent-ctl
```

Kiểm tra binary đã cài và phiên bản:
```bash
command -v agent-ctl
agent-ctl -v
# Tương đương: agent-ctl --version hoặc agent-ctl -V
```

Thông tin phiên bản gồm version, commit và ngày phát hành. Bản build local dùng
ngày commit nguồn; khi đóng gói release có thể đặt `AGENTCTL_RELEASE_DATE=YYYY-MM-DD`
trước lệnh build. Commit có hậu tố `-dirty` khi có thay đổi chưa commit.

Cập nhật binary đã cài từ nhánh `main` chính thức (cần Git, Cargo và Rust toolchain):
```bash
agent-ctl update --check  # Chỉ kiểm tra commit mới
agent-ctl update          # Tải source, build release và thay binary đang chạy
```

Updater dùng clone tạm, không thay đổi workspace hiện tại. Binary cũ chỉ được thay
sau khi build và kiểm tra `--version` thành công; thư mục chứa binary phải có quyền
ghi. Với cài đặt qua symlink, updater thay file đích của symlink.

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
docker run -d -p 127.0.0.1:8080:8080 -e TUZYGENT_API_TOKEN --name agent-core-daemon agent-core:latest
```

## Vận hành an toàn

Server yêu cầu `TUZYGENT_API_TOKEN` hoặc `--api-token` và mặc định chỉ lắng nghe
`127.0.0.1`. `/health` công khai; mọi REST API và WebSocket còn lại đều cần token.
Ví dụ khởi động:

```bash
export TUZYGENT_API_TOKEN="$(openssl rand -hex 32)"
cargo run -p agent-server -- --workspace .
curl -H "Authorization: Bearer $TUZYGENT_API_TOKEN" http://127.0.0.1:8080/api/v1/agents
```

Đặt `TUZYGENT_ALLOWED_ORIGINS=https://dashboard.example` nếu trình duyệt cần truy
cập API. Nhiều origin phân cách bằng dấu phẩy; không dùng wildcard. WebSocket
trong trình duyệt truyền token qua subprotocol (không đặt token trong URL):

```javascript
const socket = new WebSocket("wss://api.example/api/v1/ws", ["tuzygent", apiToken]);
```

Khi truy cập qua mạng, đặt server sau reverse proxy có HTTPS/WSS. Docker Compose
cũng yêu cầu biến `TUZYGENT_API_TOKEN` trước khi khởi động.

`defaults.sandbox_driver` và lựa chọn `--sandbox` thực sự bọc mọi lần chạy CLI
trong OODA, bao gồm planner và remediation. Driver lỗi sẽ trả lỗi, không chuyển
sang host. Chỉ chọn `host` khi chủ động muốn chạy trực tiếp. Bubblewrap chỉ mount
workspace, runtime hệ thống và các credential đã khai báo; không mount toàn bộ
home của host. Các executable ngoài đường dẫn hệ thống có thể cần runtime được
cài trong `/usr` hoặc một Docker image chứa đầy đủ CLI và dependency.

Với Docker sandbox, cung cấp image có các AI CLI cần dùng:

```toml
[defaults]
sandbox_driver = "docker"
sandbox_image = "your-registry/agent-engines:your-tag"
timeout_seconds = 180
```

Image sandbox mặc định là `tuzygent-engine:latest`, cần được chuẩn bị riêng.
Image daemon trong `Dockerfile` chứa server/CLI quản trị, không chứa sẵn AI CLI.
Container sandbox được đặt tên riêng và dọn khi hoàn thành, timeout hoặc hủy task.

Workflow có node lỗi sẽ báo `Failed`; node phụ thuộc sẽ không chạy. Graph có
chu trình, ID trùng hoặc edge không hợp lệ bị từ chối trước khi thực thi.
`approvalNode` trả `WaitingApproval` và chặn nhánh phụ thuộc: executor hiện chưa
có nguồn phê duyệt được xác thực hoặc API resume, nên không tự động duyệt từ
trigger payload. Tmux quản lý terminal của seat; kết quả task và channel handoff
được lấy từ lần chạy compute engine có timeout, không suy ra từ nội dung `echo`.

Khi nhúng SDK, ưu tiên `EngineInput::new()` rồi thiết lập các trường cần dùng.
Nếu tạo bằng struct literal, thêm `command_sandbox: None` và `on_event: None`; trường này chỉ được
runtime thiết lập và không được deserialize từ dữ liệu bên ngoài.


## Giao task cho Codex CLI

Adapter `CodexEngine` dùng `codex exec --json --sandbox workspace-write`, tắt
hỏi phê duyệt tương tác bằng `approval_policy="never"` và giữ nguyên sandbox OS
của OODA. Adapter không dùng cờ bỏ qua sandbox. Model lấy từ cấu hình Codex khi
không khai báo `model` trong engine hoặc seat.

Cấu hình workspace đã có `[engines.codex]`. Có thể chọn executable cụ thể bằng
`binary_path` hoặc biến `CODEX_BIN`; nếu không, adapter tìm `codex` qua PATH.
Ưu tiên executable đã cài sẵn nếu lệnh trên PATH là shim tự cập nhật của mise.
Ví dụ trên máy phát triển hiện tại:

```bash
export CODEX_BIN=/home/fox/.local/share/mise/installs/codex/0.166.0/bin/codex
cargo run -p agent-cli -- run --agent-id dev-01 --engine codex --workspace . --goal "Kiểm tra và sửa lỗi trong dự án"
```

Nếu cần dùng Codex làm mặc định, đặt `defaults.engine = "codex"`. REST API nhận:

```bash
curl http://127.0.0.1:8080/api/v1/tasks/run \
  -H "Authorization: Bearer $TUZYGENT_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"agent_id":"dev-01","engine":"codex","goal":"Kiểm tra và sửa lỗi trong dự án"}'
```

Rig seat có `engine: codex` và tùy chọn `model`; workflow agent node nhận
`data.engine = "codex"`. Tên engine không hợp lệ trả lỗi, không chuyển sang
Antigravity. Ví dụ Rig nằm tại `examples/codex-rig.yaml`:

```bash
cargo run -p agent-cli -- rig run --spec examples/codex-rig.yaml --workspace . --goal "Sửa lỗi và kiểm tra thay đổi"
```

Codex cần được cài và đăng nhập trước. `CODEX_API_KEY` có trong danh sách biến
được phép truyền vào sandbox. Nếu dùng đăng nhập ChatGPT đã lưu, mặc định chỉ
mount `~/.codex/auth.json` chỉ đọc vào `/root/.codex/auth.json`; không mount toàn
bộ thư mục cấu hình hoặc session của host. Sandbox có state riêng; nếu auth cần
refresh hoặc muốn lưu/resume session lâu dài, cần cấu hình vùng state writable
riêng và chính sách credential phù hợp. Docker image sandbox phải chứa Codex CLI
và các executable đi kèm. Không tự cài hay tự cập nhật CLI trong sandbox.

SDK xuất `CodexEngine` trong prelude. `EngineOutput.session_id` trả UUID của
session; gọi `engine.resume(&input, session_id)` để tiếp tục một session cụ thể.
Mỗi `execute()` mở session mới, không dùng `--last` chung giữa các task. Hiện
resume được cung cấp qua SDK, chưa có endpoint REST resume.

`EngineInput.on_event` nhận JSONL event dưới dạng JSON; OODA phát tiếp các event
qua `task.engine_event` trên bus/WebSocket. `on_stream_chunk` nhận văn bản agent
message hoàn tất; callback này chưa cung cấp token delta. Adapter chỉ trả thành
công khi có `turn.completed` và final message hợp lệ, đồng thời xử lý
`turn.failed`, exit code, timeout và output không hợp lệ.

Kiểm tra kết nối thật trong một Git workspace an toàn bằng ví dụ SDK:

```bash
cargo run -p agent-engine --example codex_delegate -- "$CODEX_BIN" /path/to/scratch-repo
# Thêm SESSION_UUID ở cuối để kiểm tra resume.
```

Giao thức và các cờ CLI được đối chiếu với [OpenAI Docs: Non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
và [Developer commands](https://learn.chatgpt.com/docs/developer-commands).
