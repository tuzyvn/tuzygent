# Tuzygent Autonomous Research Backlog

Danh mục đề tài nghiên cứu được trích xuất hoàn toàn động từ phân tích AST mã nguồn, chỉ số phân phối độ phức tạp thống kê, và trí nhớ lỗi thực thi runtime.

## Danh sách đề tài đang chờ giải quyết

### [P0] TOPIC-20261104-4eb926: Refactor 47 lệnh unwrap() tại crates/agent-server/src/main.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 47 lệnh unwrap() trong mã sản xuất tại crates/agent-server/src/main.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-server/src/main.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P0] TOPIC-20261104-cd0ab7: Refactor 12 lệnh unwrap() tại crates/agent-engine/src/accounts.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 12 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/accounts.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/accounts.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P0] TOPIC-20261104-5dbb90: Refactor 10 lệnh unwrap() tại crates/agent-cli/src/update.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 10 lệnh unwrap() trong mã sản xuất tại crates/agent-cli/src/update.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-cli/src/update.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-7031fa: Refactor 8 lệnh unwrap() tại crates/agent-engine/src/process.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 8 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/process.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/process.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-e312d5: Refactor 5 lệnh unwrap() tại crates/agent-runtime/src/wsrpc.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 5 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/wsrpc.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/wsrpc.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-a24108: Refactor 3 lệnh unwrap() tại crates/agent-engine/src/cli/codex.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/cli/codex.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/cli/codex.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-7cf96a: Refactor 3 lệnh unwrap() tại crates/agent-engine/src/config.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh unwrap() trong mã sản xuất tại crates/agent-engine/src/config.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-engine/src/config.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-fc7dc5: Refactor 3 lệnh unwrap() tại crates/agent-runtime/src/workflow_dag.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 3 lệnh unwrap() trong mã sản xuất tại crates/agent-runtime/src/workflow_dag.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-runtime/src/workflow_dag.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-68eb98: Refactor 1 lệnh unwrap() tại crates/agent-domain/src/identity.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-domain/src/identity.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-domain/src/identity.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-5ee0c3: Refactor 1 lệnh unwrap() tại crates/agent-memory/src/skills.rs sang cơ chế xử lý lỗi an toàn
- **Phân loại**: `Reliability / Panic Prevention`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 1 lệnh unwrap() trong mã sản xuất tại crates/agent-memory/src/skills.rs. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.
- **Tiêu chuẩn nghiệm thu**: Thay thế toàn bộ unwrap() trong crates/agent-memory/src/skills.rs bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.

### [P1] TOPIC-20261104-d55f51: Kiểm tra và cô lập 1 khối unsafe block trong crates/agent-runtime/src/processtree.rs
- **Phân loại**: `Memory Safety`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Phát hiện 1 khối unsafe trong crates/agent-runtime/src/processtree.rs. Cần kiểm tra ranh giới bộ nhớ, pointer aliasing và invariant validation.
- **Tiêu chuẩn nghiệm thu**: Bổ sung tài liệu chứng minh Safety Invariants (// SAFETY: ...) và viết test kiểm tra ranh giới cho crates/agent-runtime/src/processtree.rs.

### [P2] TOPIC-20261104-bca2f9: Phân tách cấu trúc tệp nguyên khối (1152 dòng): crates/agent-cli/src/main.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Tệp crates/agent-cli/src/main.rs có độ dài 1152 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-cli/src/main.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-1f63e7: Phân tách cấu trúc tệp nguyên khối (922 dòng): crates/agent-runtime/src/ooda_loop.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/ooda_loop.rs có độ dài 922 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/ooda_loop.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-2aa7f6: Phân tách cấu trúc tệp nguyên khối (919 dòng): crates/agent-server/src/main.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Tệp crates/agent-server/src/main.rs có độ dài 919 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-server/src/main.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-6ea655: Phân tách cấu trúc tệp nguyên khối (855 dòng): crates/agent-runtime/src/workflow_dag.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/workflow_dag.rs có độ dài 855 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/workflow_dag.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-37494f: Phân tách cấu trúc tệp nguyên khối (634 dòng): crates/agent-engine/src/accounts.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Tệp crates/agent-engine/src/accounts.rs có độ dài 634 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-engine/src/accounts.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

### [P2] TOPIC-20261104-22e857: Phân tách cấu trúc tệp nguyên khối (599 dòng): crates/agent-runtime/src/operator.rs
- **Phân loại**: `Modularity & Maintainability`
- **Trạng thái**: `PENDING_RESEARCH` ⏳
- **Lý do cần nghiên cứu**: Tệp crates/agent-runtime/src/operator.rs có độ dài 599 dòng (vượt ngưỡng P90 là 599 dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.
- **Tiêu chuẩn nghiệm thu**: Tách các struct, impl, hoặc handlers trong crates/agent-runtime/src/operator.rs thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.

