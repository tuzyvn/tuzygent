use super::TaskService;
use crate::sandbox::SandboxConfig;
use agent_domain::{GuardrailPolicy, TaskId};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

impl TaskService {
    pub fn store(&self) -> &Arc<dyn crate::task_store::TaskStore> {
        &self.store
    }

    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    pub fn resolve_workspace_dir(&self, task_id: &TaskId) -> PathBuf {
        Self::resolve_workspace(&self.workspace_dir, &task_id.0)
    }

    pub fn guardrail(&self) -> &GuardrailPolicy {
        &self.guardrail
    }

    pub fn sandbox_config(&self) -> Option<&SandboxConfig> {
        self.sandbox_config.as_ref()
    }

    pub fn concurrency_limit(&self) -> usize {
        self.max_concurrency
    }

    pub fn available_concurrency(&self) -> usize {
        self.concurrency_limiter.available_permits()
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        let limit = concurrency.max(1);
        self.max_concurrency = limit;
        self.concurrency_limiter = Arc::new(Semaphore::new(limit));
        self
    }

    pub fn with_guardrail(mut self, guardrail: GuardrailPolicy) -> Self {
        self.guardrail = guardrail;
        self
    }

    pub fn with_sandbox_config(mut self, config: SandboxConfig) -> Self {
        self.sandbox_config = Some(config);
        self
    }

    pub fn with_workspace_dir(mut self, workspace_dir: impl AsRef<Path>) -> Self {
        self.workspace_dir = workspace_dir.as_ref().to_path_buf();
        self
    }

    pub fn with_poison_threshold(mut self, threshold: usize) -> Self {
        self.poison_threshold = threshold.max(1);
        self
    }

    pub fn poison_threshold(&self) -> usize {
        self.poison_threshold
    }

    pub fn with_auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    pub fn auto_approve(&self) -> bool {
        self.auto_approve
    }
}
