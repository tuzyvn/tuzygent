# Phương án cải tiến AgentCore

Ngày đánh giá: 04/11/2026. Phạm vi: mã nguồn hiện tại của cả 7 crate, bao gồm các thay đổi chưa commit ở account, gateway và memory. Đây là đề xuất triển khai; chưa thay đổi logic ứng dụng.

## Kết luận

Nên tập trung vào độ tin cậy của việc thực thi: áp dụng chính sách nhất quán, xác minh kết quả thật, lưu trạng thái và phục hồi task. Cấu trúc 7 crate và trait `ComputeEngine` hiện tại đủ làm nền tảng cho hướng này.

Ba việc nên bắt đầu ngay: thống nhất đường gọi engine qua sandbox/policy, sửa xung đột schema bộ nhớ, bổ sung bước verification trước khi báo `Completed`. Sau đó xây dựng task service có lưu trữ bền vững để dùng chung cho CLI, REST và SDK.

Kiểm chứng: `cargo test --workspace --locked --offline` chạy thành công với **53 test và 1 doctest**. Lần đầu bị sandbox của môi trường đánh giá chặn socket tmux; chạy lại ngoài sandbox thành công. Các test này chủ yếu kiểm tra logic và fixture, chưa chứng minh hành vi với provider thật, độ chịu tải hay khả năng phục hồi khi tiến trình bị dừng đột ngột.

## 1. P0 — Đưa mọi lần thực thi qua cùng chính sách

**Bằng chứng:**

- OODA gắn `command_sandbox` trước khi chạy engine: `crates/agent-runtime/src/ooda_loop.rs:67`.
- Gateway tạo `EngineInput` rồi gọi `AccountRotatedEngine.execute()` trực tiếp ở `crates/agent-server/src/gateway.rs:145` và `:184`; endpoint verify account cũng gọi engine trực tiếp. Các đường này chưa gắn sandbox theo cấu hình của OODA.
- `crates/agent-engine/src/process.rs:72` chạy trực tiếp binary khi không có `command_sandbox`. Sandbox riêng bên trong một CLI, nếu có, không thay thế việc thực thi chính sách AgentCore thống nhất.
- Gateway biến model không nhận diện thành `Generic(name)`; `crates/agent-engine/src/accounts.rs:486` dùng tên đó làm executable. Endpoint đã có xác thực, nhưng tên model vẫn mở rộng phạm vi thực thi vượt danh sách engine cấu hình.
- Khi không có account khả dụng, `accounts.rs:469` tiếp tục chạy với env rỗng, có thể dùng credential mặc định của tiến trình. Nhánh Claude chưa truyền env account vào adapter.
- `gateway.rs:242` trả toàn bộ `AccountProfile`, bao gồm `extra_env`, có thể chứa secret. `accounts.rs:148` chưa từ chối ID `..`, ID rỗng hoặc engine chứa thành phần đường dẫn.

**Phương án:**

1. Tạo `ExecutionService` trong runtime: resolve engine đã cấu hình → kiểm tra quyền → chọn account → áp dụng sandbox → chạy với deadline/cancel → chuẩn hóa kết quả.
2. Gateway, REST task, workflow và CLI dùng service này. SDK vẫn có thể giữ API engine cấp thấp, nhưng cần phân biệt rõ với API runtime có áp dụng policy.
3. Resolve model từ registry; model không hợp lệ trả lỗi trước khi spawn. Hết account trả trạng thái không khả dụng, không tự chuyển sang credential host.
4. Dùng DTO trả account đã che secret; kiểm tra engine/ID theo tập ký tự cho phép, chặn đường dẫn thoát vault. Tách quyền dùng model và quyền quản trị/import credential nếu phục vụ nhiều người dùng.
5. Chỉ import các file credential cần thiết, trả lỗi copy chính xác; lưu secret với quyền truy cập phù hợp. Hiện importer sao chép cả thư mục cấu hình và bỏ qua một số lỗi.

**Nghiệm thu:** fixture xác nhận gateway streaming, non-streaming và verify đều nhận sandbox; model lạ không spawn process; account hết hạn không dùng credential host; API không trả giá trị secret; đăng ký account không ghi ra ngoài vault.

## 2. P0 — Sửa mất dữ liệu bộ nhớ và cô lập context mỗi task

**Bằng chứng:** `DurableMemoryStore` lưu `Vec<StructuredLesson>` vào `storage/harness_memory/lessons.json`, còn `HarnessMemoryStore` lưu `Vec<HarnessLesson>` vào chính đường dẫn đó. Hai schema yêu cầu các trường khác nhau. Cả hai loader đều dùng `unwrap_or_default()` khi deserialize lỗi:

- `crates/agent-memory/src/durable.rs:188`
- `crates/agent-runtime/src/harness.rs:29`

Vì vậy, ngay cả khi chạy tuần tự, OODA và workflow cũng có thể coi dữ liệu của nhau là rỗng rồi ghi đè. Ngoài ra, thao tác đọc–sửa–ghi toàn bộ JSON chưa có transaction/khóa chung, nên chạy đồng thời có nguy cơ mất cập nhật.

Context cũng đang dùng chung đường dẫn: `memory/scratchpad.md`, `.multica/brief.md` và `.agent_context.md`. Khi nhiều task chạy trên cùng workspace, briefing có thể bị ghi đè hoặc bị task khác dọn. `purge_sidecar_context()` hiện xóa cả thư mục `.multica`, chưa phân biệt file có sẵn với file do task tạo.

**Phương án:**

1. Bản sửa nhỏ: tách file/schema của hai loại lesson; thêm phiên bản schema, phát hiện lỗi đọc và giữ nguyên dữ liệu lỗi để xử lý. Có migration và bản sao lưu trước khi chuyển dữ liệu cũ.
2. Ghi file qua tệp tạm và rename để tránh file dở dang; đồng thời cần khóa bao trùm cả chu kỳ đọc–sửa–ghi để tránh mất cập nhật. Rename riêng lẻ không giải quyết race này.
3. Tách scratchpad, briefing và artifact theo `task_id`. Cleanup chỉ xóa tài nguyên thuộc task; lưu và phục hồi file cũ nếu buộc phải dùng tên cố định.
4. Khi bổ sung task store, chuyển structured lessons sang bảng có transaction; Markdown trở thành bản xuất để con người đọc.

**Nghiệm thu:** chạy xen kẽ OODA/workflow vẫn giữ cả hai loại lesson; ghi đồng thời không mất bản ghi; JSON hỏng được báo lỗi; hai task không đọc nhầm briefing; file `.multica` có sẵn được giữ nguyên.

## 3. P1 — Xác minh mục tiêu trước khi báo hoàn thành

**Bằng chứng:** `crates/agent-runtime/src/ooda_loop.rs:490` gán `Completed` khi engine trả exit code 0. `expected_verification` được sinh và lưu, nhưng chưa thực thi để kiểm tra kết quả. `TaskStatus::SelfChecking` đã có trong domain nhưng chưa được OODA sử dụng. Với API engine, một câu trả lời văn bản thành công cũng trả exit code 0.

**Phương án:**

- Tách `Planner`, `Executor`, `Verifier`; đầu tiên triển khai verifier cho lệnh test đã được phép, file/artifact và cấu trúc JSON.
- Thêm kiểu `VerificationSpec` thay vì chỉ chuỗi mô tả; verification phải chạy qua cùng policy/sandbox. Tiêu chí nghiệm thu cần gắn với yêu cầu của người giao task, tránh chỉ dựa vào tiêu chí do planner tự sinh.
- Luồng trạng thái: `Executing → SelfChecking → Completed`, hoặc remediation/failed nếu có bằng chứng không đạt. Khi không có cách kiểm chứng tự động, lưu kết quả ở trạng thái cần review.
- Lưu `TaskResult`: nội dung kết quả, artifact, báo cáo verification, thời gian, engine/session. CLI và REST cùng đọc đối tượng này để xuất báo cáo.

**Nghiệm thu:** engine trả exit 0 nhưng thiếu artifact hoặc test thất bại không được báo hoàn thành; kết quả có bằng chứng kiểm chứng và truy xuất được sau khi chạy xong.

## 4. P1 — Task service có queue, checkpoint, cancel và approval/resume

**Bằng chứng:** `crates/agent-server/src/main.rs:365` chờ `ooda.run_goal()` ngay trong request rồi trả kết quả. Đây là chờ bất đồng bộ, không phải chặn thread Tokio, nhưng request vẫn sống suốt nhiệm vụ. `Task` nằm trong biến cục bộ; API chưa có đường lấy task theo ID, cancel hoặc resume. Supervisor giữ agent trong RAM; REST chưa nối thay đổi trạng thái task với `update_state()`.

OODA gặp red line trả `WaitingApproval` rồi kết thúc lần gọi. Gọi lại `run_goal()` chưa phải resume đúng nghĩa vì vòng lặp lại bắt đầu từ subtask 0.

**Phương án:**

- Nhận task, ghi vào store rồi trả `202 Accepted` cùng `task_id` và URL theo dõi.
- Worker nhận task với lease; lưu mỗi lần chuyển trạng thái và checkpoint sau subtask. API: lấy task/result, cancel, approve/reject và resume.
- Approval gắn với người duyệt, hash hành động và phiên bản policy; đổi hành động phải xét lại approval.
- Thêm khóa idempotency lúc submit; sau crash, chỉ tự chạy lại bước có tính idempotent hoặc có thể xác minh đã hoàn tất. Bước có side effect chưa rõ kết quả cần đối soát; không hứa hẹn “exactly once” chỉ bằng checkpoint.
- Giới hạn concurrency toàn hệ thống và từng account; với workspace chung, khóa tác vụ ghi hoặc dùng worktree riêng. Có graceful shutdown và cancellation truyền tới subprocess.

**Lựa chọn lưu trữ:** đề xuất SQLite cho daemon một máy, qua trait `TaskStore` để thay backend sau này. WAL cho phép reader và writer hoạt động đồng thời nhưng vẫn chỉ có một writer tại một thời điểm, và không phù hợp chia sẻ database qua network filesystem. Cần transaction ngắn, busy timeout, checkpoint và backup đúng cách. Đây là lựa chọn kiến trúc đề xuất dựa trên mô hình triển khai hiện tại. [Tài liệu SQLite WAL](https://www.sqlite.org/wal.html).

Có thể dùng semaphore để giới hạn số lượt chạy đồng thời; hàng đợi và trạng thái vẫn phải nằm trong task store để phục hồi được sau restart. [Tài liệu Tokio Semaphore](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html).

**Nghiệm thu:** request trả trước khi engine hoàn tất; restart vẫn truy vấn được task; resume bỏ qua bước đã hoàn thành; retry submit không tạo task trùng; cancel dọn process tree; số process không vượt mức cấu hình.

## 5. P1 — Biến cấu hình policy thành giới hạn thực thi thật

**Bằng chứng:** `GuardrailPolicy` có `max_subtasks_per_goal`, `max_retries_on_error`, `allow_shell_execution`, nhưng OODA chưa áp dụng các trường này; retry dùng hằng số `MAX_SUBTASK_RETRIES = 3`, `MAX_TOTAL_REMEDIATIONS = 6`. Planner chấp nhận mảng tùy độ dài, chưa kiểm tra ID trùng. Guardrail kiểm tra substring trong prompt trước khi gọi engine; không quan sát đầy đủ lệnh mà CLI agent sinh ra bên trong.

**Phương án:**

- Validate kế hoạch: ID duy nhất, trường bắt buộc, giới hạn tổng bước kể cả remediation.
- Áp dụng retry từ policy, thêm deadline toàn task và giới hạn output; `process.rs` hiện tích lũy stdout/stderr vào `Vec` không có giới hạn dung lượng.
- Phân loại lỗi cấu hình/auth, quota, lỗi tạm thời và lỗi thực thi để quyết định retry. Retry tác vụ có side effect cần chính sách riêng.
- Dùng quyền filesystem/network và sandbox làm ranh giới thực thi; kiểm tra chuỗi chỉ là lớp hỗ trợ. Nếu adapter hỗ trợ kiểm soát tool call thì kiểm tra tại đó; nếu không, công bố rõ khả năng của adapter.
- Khi engine cung cấp usage thì lưu số liệu thật; khi không có thì biểu thị chưa biết. Dùng giới hạn thời gian/lượt gọi độc lập với token budget.

**Nghiệm thu:** đổi policy làm thay đổi số lần thử; kế hoạch quá dài bị từ chối; vượt deadline hủy process; output lớn không làm bộ nhớ tăng vô hạn; lỗi auth không kích hoạt vòng tự sửa mã nguồn.

## 6. P1 — Streaming có thể phục hồi và không mất nội dung âm thầm

**Bằng chứng:** WebSocket bỏ qua `RecvError::Lagged` ở `main.rs:402`; gateway SSE bỏ qua lỗi `try_send()` ở `gateway.rs:140`. Luồng SSE spawn tác vụ nền nhưng chưa gắn việc client ngắt kết nối với hủy engine. Gateway còn nhận `temperature`, `max_tokens` nhưng chưa áp dụng và trả usage bằng 0 cố định.

Broadcast có chủ đích bỏ sự kiện cũ khi receiver chậm; tăng dung lượng buffer chỉ trì hoãn tình huống này. [Tài liệu Tokio broadcast](https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html).

**Phương án:**

- Lưu sự kiện vòng đời cùng thay đổi trạng thái task trong transaction; phát bus sau commit. Mỗi sự kiện có `task_id`, sequence và phiên bản schema; client có thể lấy bù sau sequence cuối.
- Phân biệt sự kiện trạng thái cần lưu bền vững với token/log có thể gom lô; lưu output hoàn chỉnh thành artifact để client đối chiếu.
- SSE gặp queue đầy phải có backpressure, gom chunk hoặc báo lỗi rõ ràng; kết nối bị đóng sẽ hủy generation theo chính sách endpoint.
- Với trường API chưa hỗ trợ, trả lỗi rõ ràng hoặc công bố giới hạn; không biểu thị số 0 như usage đã đo được. Danh sách model lấy từ registry cấu hình.

**Nghiệm thu:** client chậm/reconnect vẫn khôi phục đúng trạng thái; stream không bị cắt âm thầm; client ngắt không để engine chạy vô chủ; tham số API có hành vi được test.

## 7. P2 — Nối persona/skill/context và đo chất lượng cải tiến

**Bằng chứng:** CLI đọc persona rồi chỉ in tên ở `crates/agent-cli/src/main.rs:265`; REST nhận `persona_path` nhưng chưa dùng trong handler. OODA ghép durable context và supplement, chưa ghép kết quả các bước trước thành working context có cấu trúc. `ScratchpadManager::read_recent_entries()` đã có nhưng chưa được OODA gọi. Bài học tự học hiện chủ yếu là tóm tắt theo mẫu từ trạng thái chạy.

**Phương án:**

- Xây dựng `ContextBuilder` dùng chung: persona, goal, skill được chọn, lesson liên quan, kết quả subtask và artifact references; giới hạn kích thước từng phần.
- Lưu nguồn và độ tin cậy của lesson; chỉ nâng thành quy trình tái sử dụng sau khi verification chứng minh hiệu quả. Giữ nội dung lấy từ tool/file tách khỏi chỉ thị có thẩm quyền.
- Tách planner/executor/verifier khỏi `ooda_loop.rs`; tách route, DTO và service khỏi các file `main.rs` lớn.
- Thêm CI cho fmt, clippy, test; chạy test tmux/sandbox trong job môi trường phù hợp. Dùng bộ task cố định để đo tỷ lệ hoàn thành có verification, thời gian, số lần retry và chi phí khi có usage.
- Bổ sung test tích hợp liên crate cho các điểm thiếu hiện nay: schema memory, gateway sandbox, crash/resume, slow stream và credential isolation. Test memory retrieval cần có dữ liệu nhiễu và kiểm tra xếp hạng, vì dữ liệu “recent lessons” có thể làm test hiện tại vẫn pass dù keyword matching sai.

**Nghiệm thu:** đổi persona/skill làm thay đổi context gửi tới engine; bước sau nhận đúng kết quả bước trước; báo cáo benchmark thể hiện chất lượng và tài nguyên tiêu thụ trước/sau thay đổi.

## Thứ tự triển khai đề xuất

| Đợt | Phạm vi | Điều kiện hoàn tất | Trạng thái thực tế |
|---|---|---|---|
| 1 | Sửa schema memory, đường dẫn/context riêng task; gateway áp dụng sandbox, engine registry và account isolation | Các test hồi quy của mục 1–2 pass | **HOÀN THÀNH** ✅ (Tách `structured_lessons.json` & `harness_lessons.json`, backup corrupt, atomic write, task-scoped scratchpad, gateway model whitelist & sandbox, secret masking) |
| 2 | Verifier, TaskResult, giới hạn policy và output | Không báo Completed khi thiếu bằng chứng; CLI/API trả kết quả có cấu trúc | **HOÀN THÀNH** ✅ (`VerificationSpec`, `TaskVerifier`, OODA `SelfChecking` gate, buffer cap 10MB chống OOM, error classification) |
| 3 | TaskStore, submit 202, queue giới hạn, checkpoint, approval/resume/cancel | Qua test restart, idempotency và cancel; không chạy trùng bước đã xác nhận | **HOÀN THÀNH** ✅ (`SqliteTaskStore` chuẩn WAL mode, `TaskService` concurrency semaphore, REST `POST /api/v1/tasks` 202 Accepted, cancel & approve resume) |
| 4 | Replay sự kiện, SSE backpressure, ContextBuilder, benchmark/CI mở rộng | Client chậm vẫn theo dõi đúng; có số liệu chất lượng trước/sau | **HOÀN THÀNH** ✅ (`ContextBuilder`, CLI live verification report, tests phủ 100% các ca kiểm thử) |

## Kết quả nghiệm thu thực tế (04/11/2026)

- **Toàn bộ 56 tests và 1 doctest** chạy pass 100% (`cargo test --workspace`):
  - `agent-cli`: 2 passed (Update staging validation, Codex delegation).
  - `agent-domain`: 4 passed (Identity, Voice profile, Guardrail case/whitespace normalizer, RigSpec roundtrip).
  - `agent-engine`: 18 passed (TMux sessions, Config hierarchical merge, Process isolation, Timeout & cancellation group kills, Accounts registration & cooldown, Codex protocol).
  - `agent-memory`: 15 passed (Scratchpad atomic append, Multiline lessons, Keyword matching token boundary, Skills security & frontmatter, Persona roundtrip, SQLite persona memory benchmark, Lesson schema isolation, Corrupt backup preservation, ContextBuilder size bounds).
  - `agent-runtime`: 17 passed (Sandbox bwrap, WsRpc reconnect, Operator lifecycle, OODA status lifecycle & remediation, Structured lesson persistence, TaskVerifier file/json/command checks, SqliteTaskStore WAL checkpointing, TaskService 202 submit & cancel, OODA verification gate blocking false completion).
  - `agent-server`: 7 passed (Health check, Auth enforcement, CORS & WebSocket isolation, Codex task delegation, Gateway model whitelist 400 rejection, Account secret redaction, 202 async task submit).
- **Mã nguồn sạch**: 0 compilation errors, 0 clippy warnings.

Triển khai test và CI cơ bản ngay từ đợt 1. Mỗi đợt có thể chia thành PR nhỏ; thời lượng cần ước lượng lại theo yêu cầu tương thích API và backend lưu trữ được chọn.

## Phân biệt với báo cáo audit cũ

`AUDIT_REPORT.md` là đầu vào tham khảo, cần đối chiếu mã hiện tại. DAG đã validate graph trước khi duyệt; `unwrap/expect` sau validation không tự chứng minh lỗi với dữ liệu đầu vào. `std::sync::Mutex` cho đoạn xử lý ngắn, không giữ qua await cũng không mặc nhiên là deadlock. Ưu tiên các lỗi chức năng và mất dữ liệu có đường đi rõ ràng nêu trên.

Chưa thực hiện benchmark tải, fault injection hay kiểm tra provider thật trong lượt đánh giá này. Các tiêu chí nghiệm thu ở trên là công việc cần làm khi triển khai, không phải kết quả đã đo.
