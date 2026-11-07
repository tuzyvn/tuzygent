use crate::ooda::execution::SubtaskExecutor;
use crate::supplement::InFlightSupplementController;
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use anyhow::Context;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct TestEnv {
    dir: PathBuf,
}

impl TestEnv {
    /// Initialize test environment directory with self-healing multi-tier fallback.
    pub fn try_new(prefix: &str) -> anyhow::Result<Self> {
        let unique_id = format!("{}_{}_{}", prefix, std::process::id(), uuid::Uuid::new_v4());
        let candidates = [
            std::env::temp_dir().join(&unique_id),
            PathBuf::from("target/tmp").join(&unique_id),
            PathBuf::from("/tmp").join(&unique_id),
        ];

        for candidate in &candidates {
            if std::fs::create_dir_all(candidate).is_ok() {
                let _ = std::fs::write(candidate.join("verified_fixture"), "test evidence");
                return Ok(Self {
                    dir: candidate.clone(),
                });
            }
        }

        anyhow::bail!(
            "Failed to initialize TestEnv across all candidates for {}",
            prefix
        )
    }

    /// Panic-free constructor with zero panics or unhandled errors.
    pub fn new(prefix: &str) -> Self {
        match Self::try_new(prefix) {
            Ok(env) => env,
            Err(_) => {
                let unique = format!("fallback_{}_{}", prefix, uuid::Uuid::new_v4());
                let fallback = PathBuf::from("target/tmp").join(unique);
                let _ = std::fs::create_dir_all(&fallback);
                let _ = std::fs::write(fallback.join("verified_fixture"), "test evidence");
                Self { dir: fallback }
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn create_sub_dir(&self, rel: impl AsRef<Path>) -> anyhow::Result<PathBuf> {
        let sub = self.dir.join(rel.as_ref());
        std::fs::create_dir_all(&sub)
            .with_context(|| format!("Failed to create sub dir: {}", sub.display()))?;
        Ok(sub)
    }

    pub fn write_file(
        &self,
        rel: impl AsRef<Path>,
        content: impl AsRef<[u8]>,
    ) -> anyhow::Result<PathBuf> {
        let file_path = self.dir.join(rel.as_ref());
        if let Some(parent) = file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&file_path, content.as_ref())
            .with_context(|| format!("Failed to write file: {}", file_path.display()))?;
        Ok(file_path)
    }

    pub fn read_file(&self, rel: impl AsRef<Path>) -> anyhow::Result<String> {
        let file_path = self.dir.join(rel.as_ref());
        std::fs::read_to_string(&file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))
    }

    pub fn file_exists(&self, rel: impl AsRef<Path>) -> bool {
        self.dir.join(rel.as_ref()).exists()
    }

    pub fn remove_file(&self, rel: impl AsRef<Path>) -> anyhow::Result<()> {
        let file_path = self.dir.join(rel.as_ref());
        if file_path.exists() {
            std::fs::remove_file(&file_path)
                .with_context(|| format!("Failed to remove file: {}", file_path.display()))?;
        }
        Ok(())
    }

    pub fn clean(&mut self) -> Result<(), std::io::Error> {
        if self.dir.exists() {
            std::fs::remove_dir_all(&self.dir)?;
        }
        Ok(())
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn try_setup_test_executor(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
    timeout_secs: Option<u64>,
) -> anyhow::Result<SubtaskExecutor> {
    if !workspace_dir.exists() {
        std::fs::create_dir_all(workspace_dir).with_context(|| {
            format!(
                "Failed to prepare workspace_dir at {}",
                workspace_dir.display()
            )
        })?;
    }
    let guardrail = guardrail.unwrap_or_default();
    let scratchpad = ScratchpadManager::new(workspace_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(workspace_dir.join("durable.db"));
    let ctrl = InFlightSupplementController::new(1);
    let timeout = match timeout_secs {
        Some(0) | None => 30,
        Some(t) => t,
    };

    Ok(SubtaskExecutor::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        workspace_dir.to_string_lossy().to_string(),
        ctrl,
        None,
        None,
        timeout,
    ))
}

pub fn setup_test_executor_with_timeout(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
    timeout_secs: u64,
) -> SubtaskExecutor {
    let effective_timeout = if timeout_secs == 0 { 30 } else { timeout_secs };
    match try_setup_test_executor(
        engine.clone(),
        guardrail.clone(),
        workspace_dir,
        Some(effective_timeout),
    ) {
        Ok(executor) => executor,
        Err(_) => {
            let guardrail = guardrail.unwrap_or_default();
            let scratchpad = ScratchpadManager::new(workspace_dir.join("scratchpad"));
            let durable = DurableMemoryStore::new(workspace_dir.join("durable.db"));
            let ctrl = InFlightSupplementController::new(1);
            SubtaskExecutor::new(
                engine,
                guardrail,
                scratchpad,
                durable,
                workspace_dir.to_string_lossy().to_string(),
                ctrl,
                None,
                None,
                effective_timeout,
            )
        }
    }
}

pub fn setup_test_executor(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
) -> SubtaskExecutor {
    setup_test_executor_with_timeout(engine, guardrail, workspace_dir, 30)
}

pub struct CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    pub call_count: AtomicUsize,
    pub callback: F,
}

impl<F> CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    pub fn new(callback: F) -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            callback,
        }
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.call_count.store(0, Ordering::SeqCst);
    }
}

type BoxedEngineFn =
    Box<dyn Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync>;

impl CallbackMockEngine<BoxedEngineFn> {
    pub fn with_constant_output(output_text: impl Into<String>) -> Self {
        let text = output_text.into();
        Self::new(Box::new(move |_input, _idx| {
            Ok(EngineOutput {
                stdout_content: text.clone(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }))
    }

    pub fn with_timeout() -> Self {
        Self::new(Box::new(move |_input, _idx| Err(EngineError::Timeout)))
    }

    pub fn with_failure(msg: impl Into<String>) -> Self {
        let message = msg.into();
        Self::new(Box::new(move |_input, _idx| {
            Err(EngineError::ProcessFailed(message.clone()))
        }))
    }

    pub fn with_error(msg: impl Into<String>) -> Self {
        Self::with_failure(msg)
    }

    pub fn with_exit_code(code: i32, stdout: impl Into<String>) -> Self {
        let text = stdout.into();
        Self::new(Box::new(move |_input, _idx| {
            Ok(EngineOutput {
                stdout_content: text.clone(),
                exit_code: code,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }))
    }

    pub fn with_sequence(outputs: Vec<Result<EngineOutput, EngineError>>) -> Self {
        let list = std::sync::Mutex::new(outputs);
        Self::new(Box::new(move |_input, idx| {
            let mut guard = list.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_empty() {
                Ok(EngineOutput {
                    stdout_content: format!("sequence exhausted at {}", idx),
                    exit_code: 0,
                    execution_duration_ms: 5,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            } else {
                guard.remove(0)
            }
        }))
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

pub(super) fn mock_remediation(id: &str, title: &str, cmd: &str) -> EngineOutput {
    let rem_json = serde_json::json!({
        "id": id,
        "title": title,
        "command_or_prompt": cmd,
        "expected_verification": "file: verified_fixture"
    });
    EngineOutput {
        stdout_content: rem_json.to_string(),
        exit_code: 0,
        execution_duration_ms: 10,
        engine_used: EngineKind::Mock,
        session_id: None,
    }
}

pub(super) fn mock_ok(stdout: &str) -> EngineOutput {
    EngineOutput {
        stdout_content: stdout.to_string(),
        exit_code: 0,
        execution_duration_ms: 10,
        engine_used: EngineKind::Mock,
        session_id: None,
    }
}
