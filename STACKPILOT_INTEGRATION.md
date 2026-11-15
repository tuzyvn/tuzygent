# TÀI LIỆU PHÁT TRIỂN: TÍCH HỢP TUZYGENT × STACKPILOT

Tài liệu này mô tả kiến trúc, cấu hình kết nối, các lưu ý vận hành và lộ trình theo mốc lịch sử cho việc kết hợp **tuzygent** (SDK `agent-core`, CLI `agent-ctl`, Daemon `agent-server`) với **StackPilot** (Local Infra Service `:49382` & MCP server).

> **Nguyên tắc tách rời (Decoupled Architecture):** Không giả định hai repo được clone cạnh nhau trên đĩa.
> - **StackPilot khai thác tuzygent** qua binary CLI đã cài trên máy (`agent-ctl` trong `PATH` / `TUZYGENT_BIN`), hoặc HTTP/WebSocket API của `agent-server`.
> - **tuzygent khai thác StackPilot** qua HTTP API (`STACKPILOT_URL`, mặc định `http://127.0.0.1:49382`) hoặc MCP server (`STACKPILOT_MCP`).

---

## 1. Phân định trách nhiệm (Separation of Concerns)

| Thành phần | Vai trò chính | Giao diện tích hợp |
| :--- | :--- | :--- |
| **tuzygent** | Bộ não điều phối multi-agent (`KernelOperator`, `AutonomousOodaLoop`), sandbox tiến trình CLI, `Sentinel Loop Guard`, bộ nhớ dài hạn (`DurableMemoryStore`), REST/WS `agent-server`. | `agent-ctl`, `agent-server` (`/api/v1/*`, `/api/v1/ws`), `agents/*.md` |
| **StackPilot** | Quản lý vòng đời dịch vụ phụ trợ cô lập theo môi trường (28 services: Postgres, Redis, Qdrant, Ollama, Kafka, Grafana/Jaeger...), cấp port động, `.env.local`, DB snapshots, Crash Doctor. | `mcp.cjs` (stdio JSON-RPC 2.0), HTTP API (`:49382`), `*.localhost` proxy |

---

## 2. Mốc lịch sử & Lộ trình phát triển (Milestones)

### Các mốc nền tảng đã hoàn thành
- **`2026-11-02` — Audit kiến trúc & bảo mật `tuzygent`**
  Hoàn tất rà soát 7 crates và các rủi ro sandbox/policy trong [AUDIT_REPORT.md](./AUDIT_REPORT.md).
- **`2026-11-04` — Kế hoạch nâng cấp độ tin cậy thực thi `tuzygent`**
  Chốt phương án P0 về thống nhất `ExecutionService`, sửa xung đột schema memory và bổ sung verification gate trong [IMPROVEMENT_PLAN.md](./IMPROVEMENT_PLAN.md) (53 unit tests + 1 doctest pass).
- **`2026-11-14` — StackPilot v0.1.0 hoàn thiện nền tảng cốt lõi**
  Đạt 28 curated services, `mcp.cjs` (6 MCP tools), DB Snapshot/Restore (`lib/snapshots.cjs`), Crash Doctor (`lib/doctor.cjs`), và `*.localhost` reverse proxy (`lib/proxy.cjs`).

### Mốc tích hợp Tuzygent × StackPilot
- **`2026-11-15` — M0: Khởi tạo thiết kế tích hợp & Roster SRE**
  - Tạo tài liệu kiến trúc tích hợp `STACKPILOT_INTEGRATION.md`.
  - Bổ sung subagent chuyên trách hạ tầng tại [agents/sre.md](./agents/sre.md) để chuẩn hóa vai trò cấp phát/chẩn đoán/dọn dẹp môi trường qua StackPilot MCP.
- **`2026-11-15` — M1: Kiểm chứng kết nối MCP & Roster thực tế (Đã xác minh)**
  - Xác minh `./target/release/agent-ctl rig status` nhận đủ 4 seat (`architect`, `developer`, `reviewer`, `sre`).
  - Xác minh `node ../StackPilot/mcp.cjs` phản hồi JSON-RPC 2.0 (`initialize`, `tools/list` đủ 6 tools, `stackpilot_list_environments` phát hiện runtime `Podman Compose` và environment `Main Fox` đang chạy `postgres` healthy tại `127.0.0.1:5432`).
  - Bổ sung cấu hình mẫu [examples/stackpilot-mcp.json](./examples/stackpilot-mcp.json) và script kiểm tra nhanh [scripts/stackpilot_mcp_smoke.sh](./scripts/stackpilot_mcp_smoke.sh).
- **`2026-11-15` — M2: DB Snapshot Checkpoint cho OODA Backtracking & Review (Đã triển khai)**
  - Bổ sung [scripts/stackpilot_checkpoint.cjs](./scripts/stackpilot_checkpoint.cjs) gọi trực tiếp `StackPilot/lib/snapshots.cjs` (`list`, `create`, `restore`, `delete`) cho Postgres, MySQL, MongoDB mà không cần phụ thuộc HTTP server.
  - Cập nhật [agents/sre.md](./agents/sre.md) yêu cầu tạo DB checkpoint trước các bước migration/test có side-effect và restore nếu OODA hoặc review từ chối.
- **`2026-11-15` — M3: Cấu hình Local LLM (Ollama) qua StackPilot (Đã tạo mẫu)**
  - Bổ sung [examples/tuzygent-stackpilot-ollama.toml](./examples/tuzygent-stackpilot-ollama.toml) cho phép chuyển `OLLAMA_HOST` và `OTEL_EXPORTER_OTLP_ENDPOINT` vào `allowed_env_vars` khi chạy với StackPilot.
  - Phần xuất telemetry từ `agent-server` / `agent-runtime` sang Prometheus/Grafana/Jaeger giữ ở bước mở rộng tiếp theo.
- **`2026-11-15` — Thử nghiệm thực tế với `vohoangtu/micro-erp` & Portable Bootstrap (Đã xác minh)**
  - Sửa tag mặc định `postgres` trong [../StackPilot/catalog.cjs](../StackPilot/catalog.cjs#L22) từ `19-alpine` sang `18-alpine`.
  - Tạo [scripts/bootstrap.sh](./scripts/bootstrap.sh) (1 lệnh setup cho máy mới: tự dò `node`, `docker`/`podman`, `bwrap`, `agy`/`claude`/`codex`/`ollama` trong `PATH` + `mise`, sinh `~/.tuzygent/engines.toml` và `~/.tuzygent/stackpilot-mcp.json`, không hardcode `/home/fox` trong repo).
  - Bỏ hardcode đường dẫn tuyệt đối trong [scripts/stackpilot_checkpoint.cjs](./scripts/stackpilot_checkpoint.cjs) và [scripts/stackpilot_mcp_smoke.sh](./scripts/stackpilot_mcp_smoke.sh).
  - Xác minh end-to-end trên `micro-erp`: dựng stack `postgres` (`18-alpine`), `valkey`, `nats` (`start -> Done`), tạo/liệt kê/restore DB snapshot `pre-migration__postgres__*.sql`, và xóa stack tạm sạch sẽ.
- **M4: Unified Local AI Dev Studio & Crash Auto-Triage**
  - Proxy `agent-server` qua `*.localhost` của StackPilot (hỗ trợ WebSocket `/api/v1/ws`).
  - Nối cảnh báo từ `Crash Doctor` (`OOMKilled`, port conflict, exit code) vào `POST /api/v1/tasks` để `sre` + `developer` tự động phân tích và đề xuất bản vá.

---

## 3. Cấu hình kết nối MCP (Cho Mốc M1)

StackPilot cung cấp MCP server không phụ thuộc npm ngoài tại `/home/fox/Projects/StackPilot/mcp.cjs` (đọc/ghi state mặc định tại `~/.stackpilot` hoặc `STACKPILOT_DATA_DIR` và gọi trực tiếp `docker` / `podman compose`).

Thêm cấu hình sau vào MCP config của AI CLI bạn dùng với `tuzygent`:

```json
{
  "mcpServers": {
    "stackpilot": {
      "command": "node",
      "args": ["/home/fox/Projects/StackPilot/mcp.cjs"]
    }
  }
}
```

### Danh sách 6 MCP Tools từ StackPilot
1. `stackpilot_list_environments` — Liệt kê các stack hiện có và trạng thái container.
2. `stackpilot_create_environment` — Tạo stack mới với `serviceIds` (vd `["node", "postgres", "redis"]`), tùy chọn `workspaceDir` và `runtimeCommand`.
3. `stackpilot_get_connections` — Lấy connection string (`DATABASE_URL`, `REDIS_URL`, ...), port map và nội dung `.env.local`.
4. `stackpilot_control_environment` — `start` / `stop` / `restart` toàn bộ stack.
5. `stackpilot_get_logs` — Đọc log container phục vụ pha **Observe** trong vòng lặp OODA.
6. `stackpilot_delete_environment` — Xóa stack tạm sau khi hoàn tất test.

---

## 4. Lưu ý quan trọng về Sandbox khi chạy chung

Theo thiết kế bảo mật của `tuzygent` (`tuzygent.toml`):
- Khi chạy với `--sandbox bubblewrap` (mặc định), tiến trình AI CLI chỉ thấy workspace hiện tại và các đường dẫn credential được khai báo `readonly` — **không** thấy `/home/fox/Projects/StackPilot/mcp.cjs`, `~/.stackpilot`, hoặc Docker/Podman socket của host.
- **Giải pháp khi chạy task cần điều khiển StackPilot**:
  - **Cách 1 (Thử nghiệm nhanh)**: Chạy riêng tác vụ điều phối hạ tầng với `--sandbox host` (chỉ dùng trên máy dev tin cậy).
  - **Cách 2 (Khuyến nghị dài hạn)**: Expose StackPilot HTTP API (trên `127.0.0.1:49382` kèm `STACKPILOT_API_TOKEN`) hoặc mở cầu nối RPC/socket có kiểm soát để agent trong sandbox gọi mà không cần cấp quyền Docker socket trực tiếp.

---

## 5. Lệnh chạy thử phối hợp Multi-Agent (`sre` + `developer`)

```bash
# Kiểm tra roster đã nhận diện subagent `sre`
cargo run -p agent-cli -- rig status
cargo run -p agent-cli -- rig coordinate --list-targets

# Thử nghiệm luồng tham vấn hạ tầng (chạy trong môi trường có quyền gọi StackPilot MCP)
cargo run -p agent-cli -- rig coordinate \
  --caller lead-operator \
  --bots sre,developer \
  --instructions "Kiểm tra các StackPilot environment hiện có, xác định service cần thiết để chạy integration test cho dự án này và đề xuất phương án kiểm thử." \
  --consult
```
