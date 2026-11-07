mod activity;
mod configuration;
mod control;
pub mod dead_letter;
pub mod dispatch;
pub mod lifecycle;
mod queries;
pub mod resume;
pub mod review;
pub mod scheduler;
mod submission;
#[cfg(test)]
mod tests;
mod worker;

pub use dead_letter::DEFAULT_POISON_THRESHOLD;
pub use scheduler::SchedulerTickReport;

use crate::sandbox::SandboxConfig;
use crate::task_store::TaskStore;
use agent_domain::GuardrailPolicy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tokio::sync::Semaphore;

/// TaskService manages task lifecycles with asynchronous background dispatch,
/// persistence, checkpointing, concurrency limits, cancellation, and human-in-the-loop approvals.
pub struct TaskService {
    pub(crate) store: Arc<dyn TaskStore>,
    pub(crate) concurrency_limiter: Arc<Semaphore>,
    pub(crate) cancellation_flags: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    pub(crate) workspace_dir: PathBuf,
    pub(crate) guardrail: GuardrailPolicy,
    pub(crate) sandbox_config: Option<SandboxConfig>,
    pub(crate) max_concurrency: usize,
    pub(crate) poison_threshold: usize,
    pub(crate) auto_approve: bool,
}

impl TaskService {
    pub fn new(
        store: Arc<dyn TaskStore>,
        workspace_dir: impl AsRef<Path>,
        max_concurrency: usize,
        guardrail: Option<GuardrailPolicy>,
        sandbox_config: Option<SandboxConfig>,
    ) -> Self {
        let max_concurrency = max_concurrency.max(1);
        Self {
            store,
            concurrency_limiter: Arc::new(Semaphore::new(max_concurrency)),
            cancellation_flags: Arc::new(RwLock::new(HashMap::new())),
            workspace_dir: workspace_dir.as_ref().to_path_buf(),
            guardrail: guardrail.unwrap_or_else(GuardrailPolicy::default_enterprise),
            sandbox_config,
            max_concurrency,
            poison_threshold: DEFAULT_POISON_THRESHOLD,
            auto_approve: false,
        }
    }
}
