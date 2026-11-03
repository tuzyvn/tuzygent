# BÁO CÁO TOÀN DIỆN: PHÂN TÍCH KIẾN TRÚC, RÀ SOÁT BẢO MẬT & ĐÁNH GIÁ CODE SMELLS

- **Dự án:** AgentCore (`/home/vohoangtu/orca/AgentCore`)
- **Engine thực thi:** Antigravity CLI Engine (`agy`)
- **Tác nhân:** `fox-agent`
- **Thời gian đánh giá:** 02/11/2026

---

## I. TỔNG QUAN KIẾN TRÚC HỆ THỐNG (ARCHITECTURE ANALYSIS)

AgentCore là một nền tảng Agentic AI hướng doanh nghiệp được xây dựng bằng Rust (2021 edition) với mô hình đa crate (Cargo Workspace), thiết kế theo mô hình vòng lặp tự chủ OODA (Observe - Orient - Decide - Act) và khả năng thích ứng tự sửa lỗi (Self-Healing).

### 1. Cấu trúc Module & Workspace
Workspace được chia thành 7 crates rõ ràng, tuân thủ nguyên lý tách biệt trách nhiệm (SoC):
1. **`crates/agent-domain`**: Chứa các cấu trúc dữ liệu cốt lõi độc lập với framework ngoài:
   - `Task`, `SubTask`, `TaskStatus`, `Priority`: Quản lý tiến trình công việc.
   - `AgentIdentity`, `AutonomyLevel`: Định danh và mức độ tự chủ của Agent.
   - `GuardrailPolicy`, `RedLineRule`: Hệ thống rào chắn chính sách hành vi an toàn.
   - `InterAgentEvent`: Cấu trúc sự kiện giao tiếp chuẩn.
2. **`crates/agent-engine`**: Lớp trừu tượng tính toán (Compute Abstraction):
   - Định nghĩa trait async `ComputeEngine`.
   - Hỗ trợ đa dạng Engine: `AntigravityEngine`, `ClaudeCodeEngine`, `CodexEngine`, `GenericCliEngine`, `OpenAiCompatibleEngine`.
   - Chuẩn hóa process runner và streaming NDJSON output.
3. **`crates/agent-memory`**: Tầng lưu trữ bền vững và bộ nhớ ngắn hạn:
   - `ScratchpadManager`: Ghi log tiến trình thực thi thời gian thực (`scratchpad.md`).
   - `DurableMemoryStore`: Bộ nhớ lâu dài đúc kết từ các bài học (`MEMORY.md`, `lessons.json`).
   - `SkillLoader`: Khả năng nạp kỹ năng động từ cấu hình YAML/Markdown.
4. **`crates/agent-runtime`**: Bộ não điều phối và thực thi:
   - `AutonomousOodaLoop`: Vòng lặp tự động phân rã mục tiêu (Decompose), kiểm tra rào chắn (Guardrail), thực thi lệnh và tự sửa sai (Backtracking Remediation).
   - `SandboxEnvironment`: Quản lý cô lập môi trường thực thi (Docker, Bubblewrap, Host).
   - `InterAgentBus`: Bus truyền thông broadcast sự kiện nội bộ đa tác nhân.
   - `WorkflowDag`: Điều phối luồng xử lý theo đồ thị có hướng không chu trình.
5. **`crates/agent-core`**: Thư viện SDK tổng hợp (Facade) cho phép tích hợp AgentCore vào các ứng dụng bên ngoài.
6. **`crates/agent-cli` (`agent-ctl`)**: Giao diện dòng lệnh chính phục vụ vận hành, kiểm thử, giám sát và phân phối nhiệm vụ.
7. **`crates/agent-server`**: Daemon server cung cấp API REST (Axum) và WebSocket thời gian thực hỗ trợ tích hợp Web Dashboard / GUI.

---

## II. RÀ SOÁT BẢO MẬT (SECURITY AUDIT)

### 🔴 Mức độ Cao (High Severity)

#### 1. Rủi ro thực thi trực tiếp trên Host khi dùng `--sandbox host`
- **Mô tả:** Khi chạy `agent-ctl run` với tham số `--sandbox host`, `SandboxEnvironment::command_sandbox` trả về `None`. Các câu lệnh OS sinh ra bởi LLM sẽ thực thi trực tiếp bằng user shell trên máy host mà không qua bất kỳ lớp cô lập cgroup, namespace hay chroot nào.
- **Rủi ro:** Nếu LLM sinh ra lệnh sai lệch hoặc bị prompt injection, mã độc có thể ghi đè file hệ thống, truy cập mã nguồn hoặc xóa dữ liệu cá nhân của user.
- **Khuyến nghị:**
  - Luôn đặt mặc định `--sandbox bubblewrap` (trên Linux) hoặc `--sandbox docker`.
  - Khi người dùng chỉ định `--sandbox host`, cần có cờ xác nhận tường minh `--dangerously-allow-host` hoặc prompt xác nhận qua terminal.

#### 2. Rào chắn Guardrail còn sơ hở trước kỹ thuật vượt rào (Bypassable Pattern Matching)
- **Mô tả:** Trong [`crates/agent-domain/src/guardrail.rs`](file:///home/vohoangtu/orca/AgentCore/crates/agent-domain/src/guardrail.rs#L55-L75), hàm `check_command_safety` chỉ kiểm tra chuỗi substring đơn giản:
  ```rust
  normalized.contains(&pattern.trim().to_lowercase())
  ```
  Các pattern hiện tại gồm `rm -rf`, `DROP TABLE|TRUNCATE`, `chmod 777|chown root`.
- **Rủi ro:** Các lệnh nguy hiểm có thể dễ dàng qua mặt:
  - Tách cờ: `rm -r -f /path` hoặc `rm --recursive --force /`
  - Đảo thứ tự: `rm -fr /path`
  - Gọi gián tiếp: `python3 -c "import shutil; shutil.rmtree('/')"` hoặc `find . -delete`
  - Phân quyền: `chmod 0777`, `chmod a+rwx`
  - Mã hóa / Shell ngầm: `echo cm0gLXJmIC8= | base64 -d | sh`
- **Khuyến nghị:**
  - Chuyển sang phân tích cú pháp AST của shell (ví dụ dùng thư viện parsing bash).
  - Sử dụng danh sách trắng (Whitelist) lệnh cho phép thay vì chỉ dựa vào Blacklist.

### 🟡 Mức độ Trung bình (Medium Severity)

#### 3. Endpoint `/api/v1/tasks/run` trong `agent-server` chạy đồng bộ (Long-polling Blocking)
- **Mô tả:** Trong [`crates/agent-server/src/main.rs`](file:///home/vohoangtu/orca/AgentCore/crates/agent-server/src/main.rs#L329), hàm xử lý HTTP giữ kết nối mở và `await ooda.run_goal(...)` cho đến khi toàn bộ nhiệm vụ hoàn tất (có thể kéo dài 5-10 phút).
- **Rủi ro:** Khi có nhiều request đồng thời, server nhanh chóng cạn kiệt tài nguyên kết nối, đồng thời dễ bị timeout ở tầng Reverse Proxy (Nginx, Traefik thường timeout sau 60s).
- **Khuyến nghị:** Chuyển sang mô hình bất đồng bộ: Nhận request -> Trả về ngay `task_id` (HTTP 202 Accepted) -> Client theo dõi tiến độ qua WebSocket `/api/v1/ws` hoặc endpoint polling `/api/v1/tasks/{id}`.

#### 4. Quản lý Secret & Credentials trong Sandbox
- **Mô tả:** Cơ chế `mount_credentials` cho phép mount file nhạy cảm từ host vào sandbox dưới dạng read-only. Nếu lệnh của agent gặp lỗi và in ra stderr, hoặc agent tóm tắt file cấu hình trong output, token có thể bị ghi lại vào log hoặc durable memory.
- **Khuyến nghị:** Bổ sung cơ chế tự động che giấu (Masking / Redaction) các chuỗi định dạng API key, bearer token trước khi ghi vào log hoặc bus sự kiện.

---

## III. ĐÁNH GIÁ CODE SMELLS & NỢ KỸ THUẬT (CODE SMELLS EVALUATION)

### 1. `agent-ctl run` nuốt kết quả chi tiết của Engine (Output Silencing)
- **Vấn đề:** Trong `crates/agent-cli/src/main.rs`, khi chạy `run`, CLI chỉ đăng ký in các sự kiện streaming (`task.subtask_executing`, `task.stream_delta`). Khi Engine Antigravity trả về kết quả ở pha hoàn thành hoặc ghi vào bộ nhớ, CLI chỉ in dòng:
  ```
  🏁 Mission Finished with Status: Completed
  🏁 Nhiệm vụ đã hoàn thành thành công!
  ```
  mà không in toàn văn output phân tích hoặc xuất ra file báo cáo `.md` cho người dùng.
- **Khắc phục:** Sau khi `ooda.run_goal` hoàn thành, CLI nên in tóm tắt kết quả hoặc tự động tạo file `REPORT_<task_id>.md` trong workspace.

### 2. Sử dụng `.unwrap()` trong logic định tuyến đồ thị (Workflow DAG)
- **Vấn đề:** Trong [`crates/agent-runtime/src/workflow_dag.rs`](file:///home/vohoangtu/orca/AgentCore/crates/agent-runtime/src/workflow_dag.rs#L826-L841), các câu lệnh như `*degrees.get_mut(neighbor).unwrap() -= 1;` có thể gây hoảng loạn (panic) nếu đồ thị có cấu trúc bất thường ngoài kiểm soát.
- **Khắc phục:** Sử dụng `if let Some(...)` hoặc trả về `Result<_, WorkflowError>`.

### 3. Đồng bộ hóa Mutex trong môi trường Async
- **Vấn đề:** Ở một số Engine Adapter (như `codex.rs`), `std::sync::Mutex` được sử dụng thay vì `tokio::sync::Mutex` hoặc atomic type.
- **Khắc phục:** Đảm bảo lock không bị giữ qua các điểm `.await` để tránh deadlock threadpool của Tokio.

---

## IV. BẢN ĐỒ LƯU TRỮ DỮ LIỆU CỦA AGENTCORE TRÊN MÁY

Khi bạn chạy mục tiêu bằng `agent-ctl run`, AgentCore lưu trữ toàn bộ trạng thái vào các thư mục sau:

| Loại dữ liệu | Vị trí lưu trữ | Nội dung |
| :--- | :--- | :--- |
| **Tiến trình thực thi** | [`AgentCore/memory/scratchpad.md`](file:///home/vohoangtu/orca/AgentCore/memory/scratchpad.md) | Ghi nhật ký từng bước SubTask, thời gian thực thi (ms) |
| **Bài học đúc kết** | [`AgentCore/memory/MEMORY.md`](file:///home/vohoangtu/orca/AgentCore/memory/MEMORY.md) | Lưu trữ các đúc kết (Lessons learned) về sự thành công/thất bại |
| **Harness Knowledge** | [`AgentCore/storage/harness_memory/lessons.json`](file:///home/vohoangtu/orca/AgentCore/storage/harness_memory/lessons.json) | Dữ liệu có cấu trúc JSON của các lần chạy trước |
| **Báo cáo phân tích** | [`AgentCore/AUDIT_REPORT.md`](file:///home/vohoangtu/orca/AgentCore/AUDIT_REPORT.md) | Báo cáo kiểm định toàn diện về Kiến trúc, Bảo mật & Code smells |
