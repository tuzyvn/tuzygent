use agent_domain::{
    GuardrailPolicy, Priority, SubTask, Task, TaskStatus, VerificationResult, VerificationSpec,
};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{InFlightSupplementController, QueueExecutionOutcome, SubtaskExecutor};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ============================================================================
// Test Environment & Mock Engine Helpers
// ============================================================================

struct TestEnv {
    dir: PathBuf,
}

impl TestEnv {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create test env dir");
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn setup_test_executor(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
) -> (SubtaskExecutor, ScratchpadManager, DurableMemoryStore) {
    let guardrail = guardrail.unwrap_or_default();
    let scratchpad = ScratchpadManager::new(workspace_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(workspace_dir.join("durable.db"));
    let supplement_ctrl = InFlightSupplementController::new(16);

    let executor = SubtaskExecutor::new(
        engine,
        guardrail,
        scratchpad.clone(),
        durable.clone(),
        workspace_dir.to_string_lossy().to_string(),
        supplement_ctrl,
        None,
        None,
        30,
    );
    (executor, scratchpad, durable)
}

struct CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    call_count: AtomicUsize,
    callback: F,
}

impl<F> CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    fn new(callback: F) -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            callback,
        }
    }
}

#[async_trait]
impl<F> ComputeEngine for CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);
        (self.callback)(input, count)
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

// ============================================================================
// Existing Domain & Memory Unit Tests
// ============================================================================

#[test]
fn test_subtask_remediation_model() {
    let parent = SubTask::new(
        "ST-1",
        "Initial Probe",
        "check service status",
        "service is up",
    );
    assert_eq!(parent.retry_count, 0);
    assert!(!parent.is_remediation());

    let remediation = SubTask::new_remediation(
        "ST-1-FIX-1",
        "Remediate Connection Timeout",
        "restart proxy service",
        &parent.id,
        "Connection refused on port 8085",
        1,
    );

    assert!(remediation.is_remediation());
    assert_eq!(remediation.parent_subtask_id.as_deref(), Some("ST-1"));
    assert_eq!(remediation.retry_count, 1);
    assert!(remediation.remediation_for_error.is_some());
}

#[tokio::test]
async fn test_durable_memory_semantic_filter() {
    let temp_dir = std::env::temp_dir().join(format!("test_mem_{}", uuid::Uuid::new_v4().simple()));
    let store = DurableMemoryStore::new(&temp_dir);

    store
        .save_lesson("Nginx port 8085 requires proxy_pass header configuration")
        .await
        .unwrap();
    store
        .save_lesson("Rust async tokio channels should specify bounded capacity")
        .await
        .unwrap();
    store
        .save_lesson("Database migrations must run before web server startup")
        .await
        .unwrap();

    let nginx_context = store
        .load_relevant_context("Nginx proxy error 502")
        .await
        .unwrap();
    assert!(nginx_context.contains("Nginx port 8085"));

    let rust_context = store
        .load_relevant_context("Rust async worker pool")
        .await
        .unwrap();
    assert!(rust_context.contains("Rust async tokio"));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[test]
fn test_task_status_lifecycle() {
    let mut task = Task::new(
        "Deploy Microservice",
        "Deploy service to k8s cluster",
        Priority::High,
    );
    assert_eq!(task.status, TaskStatus::Queued);
    assert!(task.subtasks.is_empty());

    let st1 = SubTask::new(
        "ST-1",
        "Lint configs",
        "kustomize build",
        "Zero lint errors",
    );
    task.subtasks.push(st1);
    assert_eq!(task.subtasks.len(), 1);
}

#[tokio::test]
async fn test_structured_lesson_persistence() {
    let temp_dir =
        std::env::temp_dir().join(format!("test_struct_mem_{}", uuid::Uuid::new_v4().simple()));
    let store = DurableMemoryStore::new(&temp_dir);

    let lesson = agent_memory::StructuredLesson {
        id: "lesson_test_101".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        task_title: "Fix Port Conflict".to_string(),
        trigger_error: Some("Address already in use: 3300".to_string()),
        remediation_applied: Some("fuser -k 3300/tcp".to_string()),
        general_insight: "Always check existing listeners before binding".to_string(),
        success: true,
    };

    store.save_structured_lesson(&lesson).await.unwrap();

    let loaded = store.load_structured_lessons().await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "lesson_test_101");
    assert_eq!(loaded[0].task_title, "Fix Port Conflict");
    assert!(loaded[0].success);

    // Verify MEMORY.md was also updated
    let mem_context = store.load_relevant_context("Port Conflict").await.unwrap();
    assert!(mem_context.contains("Fix Port Conflict"));
    assert!(mem_context.contains("Always check existing listeners"));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

// ============================================================================
// Subtask Queue Execution Flow Tests
// ============================================================================

/// Kiểm chứng luồng hoàn thành thành công và cập nhật context tuần tự qua các subtask.
/// ST-1 hoàn thành -> ghi vào working_context -> ST-2 nhận context ST-1 -> hoàn thành -> ST-3 nhận cả 2.
#[tokio::test]
async fn test_subtask_queue_successful_flow_and_context_updates() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_queue_success_flow");
    let workdir = env.path();

    let observed_contexts = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let observed_clone = observed_contexts.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        observed_clone
            .lock()
            .unwrap()
            .push((input.prompt_or_command.clone(), input.system_context.clone()));

        let stdout = match input.prompt_or_command.as_str() {
            "init-db" => "Database schema initialized on port 5432".to_string(),
            "run-migrations" => "Applied 3 pending SQL migrations successfully".to_string(),
            "seed-records" => "Seeded 50 admin and customer records".to_string(),
            other => format!("Unknown command: {}", other),
        };

        Ok(EngineOutput {
            stdout_content: stdout,
            exit_code: 0,
            execution_duration_ms: 15,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }));

    let (executor, scratchpad, _durable) = setup_test_executor(engine, None, workdir);

    let mut task = Task::new("Setup Service", "Prepare database and initial seed", Priority::Normal);
    task.subtasks.push(SubTask::new("ST-1", "Init Database", "init-db", "DB ready"));
    task.subtasks.push(SubTask::new("ST-2", "Run Migrations", "run-migrations", "Migrations applied"));
    task.subtasks.push(SubTask::new("ST-3", "Seed Records", "seed-records", "Data populated"));

    let mut all_verifications: Vec<VerificationResult> = Vec::new();
    let mut working_context = String::new();

    let outcome = executor
        .execute_queue(
            &mut task,
            workdir,
            "Base system prompt context",
            &mut all_verifications,
            &mut working_context,
        )
        .await?;

    // 1. Kiểm chứng kết quả tổng thể hàng đợi hoàn thành thành công
    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    assert_eq!(task.subtasks.len(), 3);

    // 2. Kiểm chứng từng subtask hoàn thành và cập nhật output excerpt
    for st in &task.subtasks {
        assert_eq!(st.status, TaskStatus::Completed);
        assert!(st.output_excerpt.is_some());
    }
    assert_eq!(
        task.subtasks[0].output_excerpt.as_deref(),
        Some("Database schema initialized on port 5432")
    );
    assert_eq!(
        task.subtasks[1].output_excerpt.as_deref(),
        Some("Applied 3 pending SQL migrations successfully")
    );
    assert_eq!(
        task.subtasks[2].output_excerpt.as_deref(),
        Some("Seeded 50 admin and customer records")
    );

    // 3. Kiểm chứng working_context được tích lũy theo định dạng ID: excerpt
    assert!(working_context.contains("ST-1: Database schema initialized on port 5432"));
    assert!(working_context.contains("ST-2: Applied 3 pending SQL migrations successfully"));
    assert!(working_context.contains("ST-3: Seeded 50 admin and customer records"));

    // 4. Kiểm chứng context truyền xuôi giữa các bước (Context Propagation to Engine)
    {
        let contexts = observed_contexts.lock().unwrap();
        assert_eq!(contexts.len(), 3);

        // ST-1 chưa có previous step results
        assert_eq!(contexts[0].0, "init-db");
        assert!(!contexts[0].1.contains("ST-1:"));

        // ST-2 nhận được kết quả của ST-1 trong system context
        assert_eq!(contexts[1].0, "run-migrations");
        assert!(contexts[1].1.contains("# Previous step results (reference data)"));
        assert!(contexts[1].1.contains("ST-1: Database schema initialized on port 5432"));
        assert!(!contexts[1].1.contains("ST-2:"));

        // ST-3 nhận được cả ST-1 lẫn ST-2 trong system context
        assert_eq!(contexts[2].0, "seed-records");
        assert!(contexts[2].1.contains("ST-1: Database schema initialized on port 5432"));
        assert!(contexts[2].1.contains("ST-2: Applied 3 pending SQL migrations successfully"));
    }

    // 5. Kiểm chứng verifications và scratchpad logs
    assert_eq!(all_verifications.len(), 3);
    assert!(all_verifications.iter().all(|v| v.passed));

    let scratchpad_logs = scratchpad.read().await?;
    assert!(scratchpad_logs.contains("SubTask ST-1 completed successfully"));
    assert!(scratchpad_logs.contains("SubTask ST-2 completed successfully"));
    assert!(scratchpad_logs.contains("SubTask ST-3 completed successfully"));

    Ok(())
}

/// Kiểm chứng luồng Backtracking & Remediation khi subtask gặp lỗi (Non-zero exit code).
/// ST-2 thất bại -> Hệ thống gọi prompt AI sinh remediation subtask -> Chèn trước ST-2 ->
/// Chạy remediation -> Cập nhật working_context -> Chạy lại ST-2 (retry 1) thành công.
#[tokio::test]
async fn test_subtask_queue_backtracking_and_remediation() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_queue_backtracking_remediation");
    let workdir = env.path();

    let retry_attempts = Arc::new(AtomicUsize::new(0));
    let retry_clone = retry_attempts.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        if input.prompt_or_command.contains("Autonomous AI Problem Solver") {
            // Hệ thống tự động sinh remediation JSON
            let rem_json = serde_json::json!({
                "id": "ST-BUILD-FIX-1",
                "title": "Install missing OpenSSL dependency",
                "command_or_prompt": "apt-get install -y libssl-dev",
                "expected_verification": "libssl headers available"
            });
            Ok(EngineOutput {
                stdout_content: rem_json.to_string(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        } else if input.prompt_or_command == "apt-get install -y libssl-dev" {
            // Thực thi bước remediation
            Ok(EngineOutput {
                stdout_content: "libssl-dev installed successfully in system".to_string(),
                exit_code: 0,
                execution_duration_ms: 20,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        } else if input.prompt_or_command == "cargo build --release" {
            let attempt = retry_clone.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                // Lần đầu: Thất bại do thiếu openssl
                Ok(EngineOutput {
                    stdout_content: "error: failed to run custom build command for openssl-sys".to_string(),
                    exit_code: 101,
                    execution_duration_ms: 30,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            } else {
                // Lần 2 (sau remediation): Kiểm tra context nhận được remediation output, sau đó thành công
                assert!(input.system_context.contains("ST-BUILD-FIX-1: libssl-dev installed successfully"));
                Ok(EngineOutput {
                    stdout_content: "Finished release [optimized] target(s) in 1.45s".to_string(),
                    exit_code: 0,
                    execution_duration_ms: 45,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            }
        } else {
            Ok(EngineOutput {
                stdout_content: format!("Executed: {}", input.prompt_or_command),
                exit_code: 0,
                execution_duration_ms: 5,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
    }));

    let (executor, _scratchpad, durable) = setup_test_executor(engine, None, workdir);

    let mut task = Task::new("Build Service", "Compile release binaries", Priority::High);
    task.subtasks.push(SubTask::new("ST-PRE", "Prepare workspace", "git status", "Clean workspace"));
    task.subtasks.push(SubTask::new("ST-BUILD", "Build Binary", "cargo build --release", "Binary compiled"));

    let mut all_verifications = Vec::new();
    let mut working_context = String::new();

    let outcome = executor
        .execute_queue(
            &mut task,
            workdir,
            "Build system root context",
            &mut all_verifications,
            &mut working_context,
        )
        .await?;

    // 1. Kết quả tổng thể thành công sau khi backtrack và remediate
    assert_eq!(outcome, QueueExecutionOutcome::Completed);

    // 2. Hàng đợi subtasks lúc này có 3 phần tử: ST-PRE, ST-BUILD-FIX-1 (được chèn vào), ST-BUILD
    assert_eq!(task.subtasks.len(), 3);

    // ST-PRE
    assert_eq!(task.subtasks[0].id, "ST-PRE");
    assert_eq!(task.subtasks[0].status, TaskStatus::Completed);

    // ST-BUILD-FIX-1 (Remediation subtask được inject trước ST-BUILD)
    let rem_subtask = &task.subtasks[1];
    assert_eq!(rem_subtask.id, "ST-BUILD-FIX-1");
    assert_eq!(rem_subtask.title, "Install missing OpenSSL dependency");
    assert_eq!(rem_subtask.status, TaskStatus::Completed);
    assert!(rem_subtask.is_remediation());
    assert_eq!(rem_subtask.parent_subtask_id.as_deref(), Some("ST-BUILD"));
    assert_eq!(rem_subtask.retry_count, 1);
    assert!(rem_subtask.remediation_for_error.is_some());

    // ST-BUILD (Original subtask sau retry)
    let orig_subtask = &task.subtasks[2];
    assert_eq!(orig_subtask.id, "ST-BUILD");
    assert_eq!(orig_subtask.status, TaskStatus::Completed);
    assert_eq!(orig_subtask.retry_count, 1);
    assert_eq!(
        orig_subtask.output_excerpt.as_deref(),
        Some("Finished release [optimized] target(s) in 1.45s")
    );

    // 3. Working context phản ánh cả bước remediation và bước retry thành công
    assert!(working_context.contains("ST-BUILD-FIX-1: libssl-dev installed successfully"));
    assert!(working_context.contains("ST-BUILD: Finished release [optimized] target(s) in 1.45s"));

    // 4. Kiểm chứng Durable Memory lưu bài học tự sửa lỗi
    let long_term_memory = durable.load_long_term_context().await?;
    assert!(long_term_memory.contains("Self-Correction"));
    assert!(long_term_memory.contains("Install missing OpenSSL dependency"));

    Ok(())
}

/// Kiểm chứng luồng Backtracking khi Verification Spec thất bại (Verification Backtracking).
/// Subtask chạy exit code 0 nhưng file artifact chưa tồn tại -> Verification thất bại ->
/// Sinh remediation subtask tạo file -> Retry lại subtask ban đầu -> Verification vượt qua -> Hoàn thành.
#[tokio::test]
async fn test_subtask_queue_verification_failure_backtracking() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_queue_verification_backtracking");
    let workdir = env.path();
    let expected_file = workdir.join("build_output.json");
    let expected_file_clone = expected_file.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        if input.prompt_or_command.contains("Autonomous AI Problem Solver") {
            let rem_json = serde_json::json!({
                "id": "ST-GEN-FIX-1",
                "title": "Create build output json",
                "command_or_prompt": "generate-build-output-json",
                "expected_verification": "build_output.json exists"
            });
            Ok(EngineOutput {
                stdout_content: rem_json.to_string(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        } else if input.prompt_or_command == "generate-build-output-json" {
            // Remediation tạo ra file đúng yêu cầu
            std::fs::write(&expected_file_clone, r#"{"status": "ok", "version": "1.0.0"}"#)
                .expect("Failed to write expected file in remediation");
            Ok(EngineOutput {
                stdout_content: "Created build_output.json successfully".to_string(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        } else {
            // Command ban đầu chạy (không tạo file ở lần 1)
            Ok(EngineOutput {
                stdout_content: "Generator script executed".to_string(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
    }));

    let (executor, _scratchpad, durable) = setup_test_executor(engine, None, workdir);

    let mut task = Task::new("Artifact Generation", "Ensure build_output.json is created", Priority::Normal);
    let mut st = SubTask::new("ST-GEN", "Generate artifact", "run-generator", "file: build_output.json");
    st.verification_spec = Some(VerificationSpec::FileExists {
        path: "build_output.json".to_string(),
        min_bytes: Some(10),
    });
    task.subtasks.push(st);

    let mut all_verifications = Vec::new();
    let mut working_context = String::new();

    let outcome = executor
        .execute_queue(
            &mut task,
            workdir,
            "Verification context",
            &mut all_verifications,
            &mut working_context,
        )
        .await?;

    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    assert_eq!(task.subtasks.len(), 2);

    // Remediation subtask
    assert_eq!(task.subtasks[0].id, "ST-GEN-FIX-1");
    assert_eq!(task.subtasks[0].status, TaskStatus::Completed);
    assert!(task.subtasks[0].is_remediation());

    // Original subtask sau khi verification vượt qua
    assert_eq!(task.subtasks[1].id, "ST-GEN");
    assert_eq!(task.subtasks[1].status, TaskStatus::Completed);
    assert_eq!(task.subtasks[1].retry_count, 1);

    // Lịch sử verification ghi nhận cả lần thất bại đầu tiên và lần thành công sau đó
    assert!(all_verifications.iter().any(|v| !v.passed));
    assert!(all_verifications.iter().any(|v| v.passed));

    // Durable memory ghi nhận bài học tự sửa lỗi verification
    let memory_context = durable.load_long_term_context().await?;
    assert!(memory_context.contains("Verification Self-Correction"));

    Ok(())
}

/// Kiểm chứng luồng Backtracking khi số lần thử lại vượt quá giới hạn (Exhausted Retries).
/// Khi subtask liên tục thất bại và retry_count vượt ngưỡng `max_retries_on_error`,
/// hàng đợi phải dừng lại và trả về `QueueExecutionOutcome::Failed`, task chuyển `TaskStatus::Failed`.
#[tokio::test]
async fn test_subtask_queue_backtracking_exhausted_retries() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_queue_exhausted_retries");
    let workdir = env.path();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        if input.prompt_or_command.contains("Autonomous AI Problem Solver") {
            let rem_json = serde_json::json!({
                "id": "ST-FAIL-RETRY",
                "title": "Attempt fix",
                "command_or_prompt": "apply-fix",
                "expected_verification": "Error mitigated"
            });
            Ok(EngineOutput {
                stdout_content: rem_json.to_string(),
                exit_code: 0,
                execution_duration_ms: 5,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        } else if input.prompt_or_command == "apply-fix" {
            Ok(EngineOutput {
                stdout_content: "Fix applied, but root problem persists".to_string(),
                exit_code: 0,
                execution_duration_ms: 5,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        } else {
            // Persistent fatal failure
            Ok(EngineOutput {
                stdout_content: "Fatal kernel error: out of memory".to_string(),
                exit_code: 137,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
    }));

    // Cấu hình giới hạn retry = 2
    let guardrail = GuardrailPolicy {
        max_retries_on_error: 2,
        ..Default::default()
    };

    let (executor, _scratchpad, durable) = setup_test_executor(engine, Some(guardrail), workdir);

    let mut task = Task::new("OOM Prone Task", "Task that keeps failing", Priority::High);
    task.subtasks.push(SubTask::new("ST-OOM", "Heavy compute", "run-heavy", "Success"));

    let mut all_verifications = Vec::new();
    let mut working_context = String::new();

    let outcome = executor
        .execute_queue(
            &mut task,
            workdir,
            "OOM context",
            &mut all_verifications,
            &mut working_context,
        )
        .await?;

    // Hàng đợi dừng với kết quả Failed do cạn kiệt retry
    match outcome {
        QueueExecutionOutcome::Failed { error } => {
            assert!(error.contains("137"));
        }
        other => panic!("Expected QueueExecutionOutcome::Failed, got {:?}", other),
    }

    // Task và subtask đều ở trạng thái Failed
    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    let failed_st = task.subtasks.iter().find(|s| s.id == "ST-OOM").unwrap();
    assert!(matches!(failed_st.status, TaskStatus::Failed { .. }));
    assert_eq!(failed_st.retry_count, 2);

    // Durable memory ghi nhận bài học exhausted retries
    let memory_context = durable.load_long_term_context().await?;
    assert!(memory_context.contains("Exhausted retries in subtask"));

    Ok(())
}
