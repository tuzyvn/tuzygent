# Tuzygent Autonomous Research Backlog

Danh mục đề tài nghiên cứu được trích xuất hoàn toàn động từ phân tích AST mã nguồn, chỉ số phân phối độ phức tạp thống kê, và trí nhớ lỗi thực thi runtime.

## Danh sách đề tài đang chờ giải quyết

### [P0] TOPIC-20261104-4eb926: Refactor 47 lệnh unwrap() tại crates/agent-server/src/main.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 15:48:08 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 47 lệnh unwrap() trong mã sản xuất tại crates/agent-server/src/main.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-server/src/main.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P0] TOPIC-20261104-cd0ab7: Refactor 12 lệnh unwrap() tại crates/agent-engine/src/accounts.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 15:58:56 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 12 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/accounts.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/accounts.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P0] TOPIC-20261104-5dbb90: Refactor 10 lệnh unwrap() tại crates/agent-cli/src/update.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:03:23 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 10 lệnh unwrap() trong mã sản xuất tại crates/agent-cli/src/update.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-cli/src/update.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-7031fa: Refactor 8 lệnh unwrap() tại crates/agent-engine/src/process.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:07:33 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 8 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/process.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/process.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-e312d5: Refactor 5 lệnh unwrap() tại crates/agent-runtime/src/wsrpc.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:21:40 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 5 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/wsrpc.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/wsrpc.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-a24108: Refactor 3 lệnh unwrap() tại crates/agent-engine/src/cli/codex.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:24:21 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/cli/codex.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/cli/codex.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-7cf96a: Refactor 3 lệnh unwrap() tại crates/agent-engine/src/config.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:27:30 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/config.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/config.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-fc7dc5: Refactor 3 lệnh unwrap() tại crates/agent-runtime/src/workflow_dag.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:32:55 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/workflow_dag.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/workflow_dag.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-68eb98: Refactor 1 lệnh unwrap() tại crates/agent-domain/src/identity.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:35:59 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-domain/src/identity.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-domain/src/identity.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-5ee0c3: Refactor 1 lệnh unwrap() tại crates/agent-memory/src/skills.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:41:04 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-memory/src/skills.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-memory/src/skills.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-d55f51: Kiểm tra và cô lập 1 khối unsafe block trong crates/agent-runtime/src/processtree.rs
- **Phân loại**: `Memory Safety`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 16:47:56 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 khối unsafe trong crates/agent-runtime/src/processtree.rs. Cần kiểm tra ranh giới bộ nhớ, pointer aliasing và invariant validation.
- **Tiêu chuẩn nghiệm thu**: Bổ sung tài liệu chứng minh Safety Invariants (// SAFETY: ...) và viết test kiểm tra ranh giới cho crates/agent-runtime/src/processtree.rs.

### [P2] TOPIC-20261104-bca2f9: Phân tách cấu trúc tệp nguyên khối (1152 dòng): crates/agent-cli/src/main.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:02:02 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-cli/src/main.rs có độ dài 1152 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-cli/src/main.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-1f63e7: Phân tách cấu trúc tệp nguyên khối (922 dòng): crates/agent-runtime/src/ooda_loop.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:13:52 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda_loop.rs có độ dài 922 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda_loop.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-2aa7f6: Phân tách cấu trúc tệp nguyên khối (919 dòng): crates/agent-server/src/main.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:24:26 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-server/src/main.rs có độ dài 919 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-server/src/main.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-6ea655: Phân tách cấu trúc tệp nguyên khối (855 dòng): crates/agent-runtime/src/workflow_dag.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:32:27 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/workflow_dag.rs có độ dài 855 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/workflow_dag.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-37494f: Phân tách cấu trúc tệp nguyên khối (634 dòng): crates/agent-engine/src/accounts.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:38:56 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-engine/src/accounts.rs có độ dài 634 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-engine/src/accounts.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-22e857: Phân tách cấu trúc tệp nguyên khối (599 dòng): crates/agent-runtime/src/operator.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:46:18 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/operator.rs có độ dài 599 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/operator.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P0] TOPIC-20261104-7a8c93: Refactor 46 lệnh unwrap() tại crates/agent-server/src/tests.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:50:18 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 46 lệnh unwrap() trong mã sản xuất tại crates/agent-server/src/tests.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-server/src/tests.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P0] TOPIC-20261104-9e8a6c: Refactor 12 lệnh unwrap() tại crates/agent-runtime/src/workflow_dag/tests.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 17:58:44 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 12 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/workflow_dag/tests.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/workflow_dag/tests.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P0] TOPIC-20261104-374fbc: Khử 12 điểm gọi panic!() tường minh trong crates/agent-runtime/src/wsrpc.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:02:13 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 12 lệnh panic!() trong crates/agent-runtime/src/wsrpc.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-runtime/src/wsrpc.rs thành kiểu trả về Result<T, E>.

### [P0] TOPIC-20261104-330960: Khử 2 điểm gọi panic!() tường minh trong crates/agent-engine/src/accounts/vault.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:05:51 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 2 lệnh panic!() trong crates/agent-engine/src/accounts/vault.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-engine/src/accounts/vault.rs thành kiểu trả về Result<T, E>.

### [P1] TOPIC-20261104-cb64ee: Refactor 7 lệnh unwrap() tại crates/agent-engine/src/accounts/vault.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:08:51 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 7 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/accounts/vault.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/accounts/vault.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-ec2a79: Refactor 2 lệnh unwrap() tại crates/agent-memory/src/skills.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:14:51 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 2 lệnh unwrap() trong mã sản xuất tại crates/agent-memory/src/skills.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-memory/src/skills.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-cc06aa: Refactor 1 lệnh unwrap() tại crates/agent-server/src/main.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:18:46 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-server/src/main.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-server/src/main.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-fd0234: Triệt tiêu lỗi hệ thống lặp lại (clippy::lint_rules)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:20:52 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): 'ved.unwrap());
    |

    Checking agent-core v0.1.0 (/home/fox/tuzygent/crates/'. Giải pháp tạm thời hiện tại là: 'Fix warnings or explicitly allow with #[allow(...)] and inline justification'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý lint_rules để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P1] TOPIC-20261104-900b56: Triệt tiêu lỗi hệ thống lặp lại (research_mission::TOPIC-20261104-e312d5)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:29:14 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): 'Mission exited with non-zero code 1'. Giải pháp tạm thời hiện tại là: 'Kiểm tra lại compilation errors và logic test'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý TOPIC-20261104-e312d5 để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P1] TOPIC-20261104-e851a0: Triệt tiêu lỗi hệ thống lặp lại (research_mission::TOPIC-20261104-d55f51)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:36:30 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): 'Mission exited with non-zero code 1'. Giải pháp tạm thời hiện tại là: 'Kiểm tra lại compilation errors và logic test'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý TOPIC-20261104-d55f51 để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P1] TOPIC-20261104-4c7d3d: Triệt tiêu lỗi hệ thống lặp lại (cargo_test::workspace)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 18:49:24 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): 'ning unittests src/lib.rs (target/debug/deps/agent_core-1441647b57ebddde)
     R'. Giải pháp tạm thời hiện tại là: 'Run cargo test -p <failed_crate> with verbose logging to isolate regression'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý workspace để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P1] TOPIC-20261104-8bcc6a: Triệt tiêu lỗi hệ thống lặp lại (clippy::lint_rules)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:09:34 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): ' `-D warnings`
   = help: to override `-D warnings` add `#[allow(ambiguous_glob_'. Giải pháp tạm thời hiện tại là: 'Fix warnings or explicitly allow with #[allow(...)] and inline justification'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý lint_rules để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P2] TOPIC-20261104-17b240: Phân tách cấu trúc tệp nguyên khối (757 dòng): crates/agent-runtime/src/workflow_dag/orchestrator.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:19:14 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/workflow_dag/orchestrator.rs có độ dài 757 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/workflow_dag/orchestrator.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-7aa6aa: Phân tách cấu trúc tệp nguyên khối (691 dòng): crates/agent-runtime/src/ooda/execution.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:31:02 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda/execution.rs có độ dài 691 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda/execution.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-171b7d: Phân tách cấu trúc tệp nguyên khối (549 dòng): crates/agent-engine/src/accounts/vault.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:37:40 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-engine/src/accounts/vault.rs có độ dài 549 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-engine/src/accounts/vault.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-6d89e9: Phân tách cấu trúc tệp nguyên khối (497 dòng): crates/agent-server/src/dashboard_html.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:41:08 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-server/src/dashboard_html.rs có độ dài 497 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-server/src/dashboard_html.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-58b027: Phân tách cấu trúc tệp nguyên khối (487 dòng): crates/agent-runtime/src/sandbox.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:47:31 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/sandbox.rs có độ dài 487 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/sandbox.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-502f5f: Phân tách cấu trúc tệp nguyên khối (387 dòng): crates/agent-server/src/gateway.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:53:00 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-server/src/gateway.rs có độ dài 387 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-server/src/gateway.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-5d08ca: Phân tách cấu trúc tệp nguyên khối (371 dòng): crates/agent-engine/src/config.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 19:59:55 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-engine/src/config.rs có độ dài 371 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-engine/src/config.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-c1aa8f: Phân tách cấu trúc tệp nguyên khối (364 dòng): crates/agent-server/src/tests.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:04:30 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-server/src/tests.rs có độ dài 364 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-server/src/tests.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-2987fe: Phân tách cấu trúc tệp nguyên khối (362 dòng): crates/agent-memory/src/persona_memory.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:08:04 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-memory/src/persona_memory.rs có độ dài 362 dòng (vượt ngưỡng P90 là 362 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-memory/src/persona_memory.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P0] TOPIC-20261104-7054d0: Khử 4 điểm gọi panic!() tường minh trong crates/agent-memory/src/skills.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:10:50 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 4 lệnh panic!() trong crates/agent-memory/src/skills.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-memory/src/skills.rs thành kiểu trả về Result<T, E>.

### [P1] TOPIC-20261104-1e17ee: Refactor 7 lệnh unwrap() tại crates/agent-server/src/gateway/mod.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:20:13 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 7 lệnh unwrap() trong mã sản xuất tại crates/agent-server/src/gateway/mod.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-server/src/gateway/mod.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-e3ab1b: Refactor 6 lệnh unwrap() tại crates/agent-server/src/gateway/types.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:25:11 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 6 lệnh unwrap() trong mã sản xuất tại crates/agent-server/src/gateway/types.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-server/src/gateway/types.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-46292c: Refactor 2 lệnh unwrap() tại crates/agent-engine/src/accounts/vault/mod.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:29:32 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 2 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/accounts/vault/mod.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/accounts/vault/mod.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-343092: Refactor 2 lệnh unwrap() tại crates/agent-runtime/src/ooda/execution/runner.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:33:37 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 2 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/ooda/execution/runner.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/ooda/execution/runner.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-783b3a: Refactor 2 lệnh unwrap() tại crates/agent-runtime/src/processtree.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:36:14 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 2 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/processtree.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/processtree.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-a6cb11: Refactor 1 lệnh unwrap() tại crates/agent-runtime/src/workflow_dag/tests.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:38:28 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/workflow_dag/tests.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/workflow_dag/tests.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-89ca1e: Triệt tiêu lỗi hệ thống lặp lại (research_mission::TOPIC-20261104-8bcc6a)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:47:20 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): 'Mission exited with non-zero code 1'. Giải pháp tạm thời hiện tại là: 'Kiểm tra lại compilation errors và logic test'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý TOPIC-20261104-8bcc6a để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P2] TOPIC-20261104-9c9547: Phân tách cấu trúc tệp nguyên khối (559 dòng): crates/agent-runtime/src/workflow_dag/node_executor.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 21:03:22 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/workflow_dag/node_executor.rs có độ dài 559 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/workflow_dag/node_executor.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-567c6a: Phân tách cấu trúc tệp nguyên khối (529 dòng): crates/agent-runtime/src/ooda/execution/runner.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 21:24:29 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda/execution/runner.rs có độ dài 529 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda/execution/runner.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-636c4a: Phân tách cấu trúc tệp nguyên khối (500 dòng): crates/agent-runtime/src/workflow_dag/tests.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 21:35:14 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/workflow_dag/tests.rs có độ dài 500 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/workflow_dag/tests.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-d0ed5f: Phân tách cấu trúc tệp nguyên khối (434 dòng): crates/agent-engine/src/accounts/vault/mod.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 21:43:47 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-engine/src/accounts/vault/mod.rs có độ dài 434 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-engine/src/accounts/vault/mod.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-1a1413: Phân tách cấu trúc tệp nguyên khối (426 dòng): crates/agent-runtime/src/wsrpc.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 21:47:10 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/wsrpc.rs có độ dài 426 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/wsrpc.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-c0f52b: Phân tách cấu trúc tệp nguyên khối (379 dòng): crates/agent-runtime/src/sandbox/env.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:02:59 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/sandbox/env.rs có độ dài 379 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/sandbox/env.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-f83c33: Phân tách cấu trúc tệp nguyên khối (359 dòng): crates/agent-memory/src/persona_memory/service.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:09:09 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-memory/src/persona_memory/service.rs có độ dài 359 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-memory/src/persona_memory/service.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P1] TOPIC-20261104-55013c: Triệt tiêu lỗi hệ thống lặp lại (research_mission::TOPIC-20261104-1e17ee)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 20:20:13 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): 'Mission exited with non-zero code 1'. Giải pháp tạm thời hiện tại là: 'Kiểm tra lại compilation errors và logic test'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý TOPIC-20261104-1e17ee để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P0] TOPIC-20261104-cf3fb2: Khử 3 điểm gọi panic!() tường minh trong crates/agent-runtime/src/ooda/execution/runner.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:15:56 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh panic!() trong crates/agent-runtime/src/ooda/execution/runner.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-runtime/src/ooda/execution/runner.rs thành kiểu trả về Result<T, E>.

### [P0] TOPIC-20261104-337726: Khử 2 điểm gọi panic!() tường minh trong crates/agent-runtime/src/processtree.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:18:19 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 2 lệnh panic!() trong crates/agent-runtime/src/processtree.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-runtime/src/processtree.rs thành kiểu trả về Result<T, E>.

### [P0] TOPIC-20261104-f4e594: Khử 1 điểm gọi panic!() tường minh trong crates/agent-runtime/src/ooda/execution/remediation.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:20:58 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh panic!() trong crates/agent-runtime/src/ooda/execution/remediation.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-runtime/src/ooda/execution/remediation.rs thành kiểu trả về Result<T, E>.

### [P0] TOPIC-20261104-1b685a: Khử 1 điểm gọi panic!() tường minh trong crates/agent-runtime/src/ooda/execution/verification.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:23:31 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh panic!() trong crates/agent-runtime/src/ooda/execution/verification.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-runtime/src/ooda/execution/verification.rs thành kiểu trả về Result<T, E>.

### [P0] TOPIC-20261104-40b31a: Khử 1 điểm gọi panic!() tường minh trong crates/agent-server/src/gateway/types.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:26:19 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh panic!() trong crates/agent-server/src/gateway/types.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-server/src/gateway/types.rs thành kiểu trả về Result<T, E>.

### [P1] TOPIC-20261104-b21124: Refactor 8 lệnh unwrap() tại crates/agent-runtime/src/ooda/execution/runner.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:32:33 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 8 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/ooda/execution/runner.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/ooda/execution/runner.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-69aa98: Refactor 4 lệnh unwrap() tại crates/agent-runtime/src/sandbox/context.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:34:48 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 4 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/sandbox/context.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/sandbox/context.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-eb51fa: Refactor 4 lệnh unwrap() tại crates/agent-runtime/src/sandbox/env.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:37:08 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 4 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/sandbox/env.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/sandbox/env.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-b19049: Refactor 3 lệnh unwrap() tại crates/agent-runtime/src/sandbox/workspace.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:39:41 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/sandbox/workspace.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/sandbox/workspace.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-17caed: Refactor 1 lệnh unwrap() tại crates/agent-engine/src/accounts/vault/tests.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:42:33 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/accounts/vault/tests.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/accounts/vault/tests.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-1ac134: Refactor 1 lệnh unwrap() tại crates/agent-runtime/src/ooda/execution/git.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:45:02 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/ooda/execution/git.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/ooda/execution/git.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-92d6ab: Refactor 1 lệnh unwrap() tại crates/agent-runtime/src/ooda/execution/safety.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:48:55 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/ooda/execution/safety.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/ooda/execution/safety.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-ed983d: Refactor 1 lệnh unwrap() tại crates/agent-runtime/src/ooda/execution/verification.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 22:51:37 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/ooda/execution/verification.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/ooda/execution/verification.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P2] TOPIC-20261104-049d13: Phân tách cấu trúc tệp nguyên khối (864 dòng): crates/agent-runtime/src/ooda/execution/runner.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 23:04:13 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda/execution/runner.rs có độ dài 864 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda/execution/runner.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-575acc: Phân tách cấu trúc tệp nguyên khối (398 dòng): crates/agent-runtime/src/ooda/execution/remediation.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 23:10:03 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda/execution/remediation.rs có độ dài 398 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda/execution/remediation.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-c658ea: Phân tách cấu trúc tệp nguyên khối (478 dòng): crates/agent-runtime/src/ooda/execution/runner_tests.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 23:23:15 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda/execution/runner_tests.rs có độ dài 478 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda/execution/runner_tests.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.
- **Kết quả thực hiện**: Phân rã hoàn chỉnh thành thư mục module `runner_tests/` gồm 5 tệp chuyên biệt:
  - `fixtures.rs` (94 dòng): TestEnv, setup_test_executor, CallbackMockEngine.
  - `guardrail.rs` (80 dòng): Kiểm thử các chính sách bảo vệ guardrail redline và shell execution.
  - `lifecycle.rs` (159 dòng): Kiểm thử vòng đời thực thi subtask (resume, completed, single success).
  - `verification.rs` (159 dòng): Kiểm thử xác minh subtask thất bại và kích hoạt remediation retry.
  - `mod.rs` (6 dòng): Khai báo submodule.
  Tất cả các tệp đều <= 159 dòng (dưới ngưỡng P90 350 dòng), bổ sung test case TDD xác thực cấu trúc và vượt qua 100% harness benchmark.

### [P2] TOPIC-20261104-ed9d9b: Phân tách cấu trúc tệp nguyên khối (372 dòng): crates/agent-runtime/src/ooda/execution/runner.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 23:41:43 UTC)
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda/execution/runner.rs có độ dài 372 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda/execution/runner.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P0] TOPIC-20261104-f037ed: Khử 1 điểm gọi panic!() tường minh trong crates/agent-runtime/src/ooda/execution/runner/step.rs
- **Phân loại**: `Reliability / Crash Immunity`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 23:44:37 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh panic!() trong crates/agent-runtime/src/ooda/execution/runner/step.rs. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.
- **Tiêu chuẩn nghiệm thu**: Chuyển đổi các nhánh panic!() trong crates/agent-runtime/src/ooda/execution/runner/step.rs thành kiểu trả về Result<T, E>.

### [P1] TOPIC-20261104-e8fd14: Refactor 2 lệnh unwrap() tại crates/agent-runtime/src/ooda/execution/runner/step.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-04 23:49:58 UTC)
- **Lý do cần nghiên cứu**: Phát hiện 2 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/ooda/execution/runner/step.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/ooda/execution/runner/step.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-6ca21d: Triệt tiêu lỗi hệ thống lặp lại (research_mission::TOPIC-20261104-c658ea)
- **Phân loại**: `Self-Healing Root Cause`
- **Trạng thái**: `COMPLETED` ✅ (2026-11-05 00:40:28 UTC)
- **Lý do cần nghiên cứu**: Hệ thống ghi nhận lỗi tái diễn (1 lần): 'Mission exited with non-zero code 1'. Giải pháp tạm thời hiện tại là: 'Kiểm tra lại compilation errors và logic test'. Cần sửa tận gốc trong mã nguồn.
- **Tiêu chuẩn nghiệm thu**: Refactor mã nguồn xử lý TOPIC-20261104-c658ea để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.

### [P2] TOPIC-20261104-60e652: Phân tách cấu trúc tệp nguyên khối (361 dòng): crates/agent-runtime/src/ooda/execution/runner/step.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda/execution/runner/step.rs có độ dài 361 dòng (vượt ngưỡng P90 là 350 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda/execution/runner/step.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

