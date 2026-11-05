use crate::ooda::execution::SubtaskExecutor;
use crate::supplement::InFlightSupplementController;
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct TestEnv {
    dir: PathBuf,
}

impl TestEnv {
    pub fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("Failed to create test temp dir");
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn setup_test_executor(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
) -> SubtaskExecutor {
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
        30,
    )
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
