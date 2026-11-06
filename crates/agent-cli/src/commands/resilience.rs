use crate::cli::Cli;
use crate::fault::{CliExecutionError, CliFaultPolicy};
use crate::resilience::ResilientCliRunner;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

/// Resilient command dispatcher for `agent-cli::commands` providing comprehensive
/// chaos engineering fault tolerance, bounded memory allocation, and recovery mechanisms.
#[derive(Clone)]
pub struct ResilientCommandDispatcher {
    runner: ResilientCliRunner,
}

impl Default for ResilientCommandDispatcher {
    fn default() -> Self {
        Self::new(ResilientCliRunner::default())
    }
}

impl ResilientCommandDispatcher {
    pub fn new(runner: ResilientCliRunner) -> Self {
        Self { runner }
    }

    pub fn with_policy(policy: CliFaultPolicy, max_retries: usize) -> Self {
        Self {
            runner: ResilientCliRunner::new(policy, max_retries),
        }
    }

    pub fn with_audit_workspace(mut self, path: impl AsRef<Path>) -> Self {
        self.runner = self.runner.with_audit_workspace(path);
        self
    }

    pub fn with_max_capacity(mut self, max: usize) -> Self {
        self.runner = self.runner.with_max_capacity(max);
        self
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.runner.max_capacity()
    }

    pub fn memory_usage_bytes(&self) -> usize {
        self.runner.memory_usage_bytes()
    }

    pub fn runner(&self) -> &ResilientCliRunner {
        &self.runner
    }

    pub fn runner_mut(&mut self) -> &mut ResilientCliRunner {
        &mut self.runner
    }

    pub fn recovered_count(&self) -> usize {
        self.runner.recovered_count()
    }

    pub fn total_executed(&self) -> usize {
        self.runner.total_executed()
    }

    pub fn failed_count(&self) -> usize {
        self.runner.failed_count()
    }

    pub fn rejected_count(&self) -> usize {
        self.runner.rejected_count()
    }

    pub fn panics_caught_count(&self) -> usize {
        self.runner.panics_caught_count()
    }

    pub fn reset_counts(&mut self) {
        self.runner.reset_counts();
    }

    pub async fn dispatch(&mut self, cli: Cli) -> Result<(), CliExecutionError> {
        self.runner.run_command_resilient(cli).await
    }

    pub async fn dispatch_operation<F, Fut, T>(
        &mut self,
        action_name: &str,
        op: F,
    ) -> Result<T, CliExecutionError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, CliExecutionError>>,
    {
        self.runner.run_resilient(action_name, op).await
    }

    pub async fn dispatch_batch_operations<T: Clone>(
        &mut self,
        operations: &[(&str, Result<T, CliExecutionError>)],
    ) -> Vec<Result<T, CliExecutionError>> {
        self.runner.execute_batch_operations(operations).await
    }

    pub async fn dispatch_batch_commands(
        &mut self,
        commands: &[Cli],
    ) -> Vec<Result<(), CliExecutionError>> {
        self.runner.execute_batch_commands(commands).await
    }
}

/// Sharded resilient command dispatcher mitigating high-load lock contention
/// across concurrent worker tasks in `agent-cli::commands`.
#[derive(Clone)]
pub struct ShardedResilientCommandDispatcher {
    shards: Arc<Vec<tokio::sync::Mutex<ResilientCommandDispatcher>>>,
    num_shards: usize,
    total_capacity: Option<usize>,
}

impl Default for ShardedResilientCommandDispatcher {
    fn default() -> Self {
        Self::new(CliFaultPolicy::none(), 3, 16)
    }
}

impl ShardedResilientCommandDispatcher {
    pub fn new(policy: CliFaultPolicy, max_retries: usize, num_shards: usize) -> Self {
        let num_shards = num_shards.max(1);
        let mut shards = Vec::with_capacity(num_shards);
        for _ in 0..num_shards {
            shards.push(tokio::sync::Mutex::new(
                ResilientCommandDispatcher::with_policy(policy.clone(), max_retries),
            ));
        }
        Self {
            shards: Arc::new(shards),
            num_shards,
            total_capacity: None,
        }
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.total_capacity = Some(max_capacity);
        let per_shard = (max_capacity / self.num_shards).max(1) + 1;
        if let Some(shards) = Arc::get_mut(&mut self.shards) {
            for shard in shards.iter_mut() {
                let disp = shard.get_mut();
                *disp = disp.clone().with_max_capacity(per_shard);
            }
        }
        self
    }

    pub fn with_audit_workspace(mut self, path: impl AsRef<Path>) -> Self {
        let p = path.as_ref();
        if let Some(shards) = Arc::get_mut(&mut self.shards) {
            for shard in shards.iter_mut() {
                let disp = shard.get_mut();
                *disp = disp.clone().with_audit_workspace(p);
            }
        }
        self
    }

    pub fn shard_idx(&self, key: &str) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn shard(&self, idx: usize) -> &tokio::sync::Mutex<ResilientCommandDispatcher> {
        &self.shards[idx % self.num_shards]
    }

    pub fn shard_for(&self, key: &str) -> &tokio::sync::Mutex<ResilientCommandDispatcher> {
        self.shard(self.shard_idx(key))
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    pub fn total_capacity(&self) -> Option<usize> {
        self.total_capacity
    }

    pub async fn dispatch_operation<F, Fut, T>(
        &self,
        action_name: &str,
        op: F,
    ) -> Result<T, CliExecutionError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, CliExecutionError>>,
    {
        let shard = self.shard_for(action_name);
        let mut guard = shard.lock().await;
        guard.dispatch_operation(action_name, op).await
    }

    pub async fn dispatch(&self, cli: Cli) -> Result<(), CliExecutionError> {
        let key = format!("{:?}", cli.command);
        let shard = self.shard_for(&key);
        let mut guard = shard.lock().await;
        guard.dispatch(cli).await
    }

    pub async fn total_executed(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.total_executed();
        }
        total
    }

    pub async fn recovered_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.recovered_count();
        }
        total
    }

    pub async fn failed_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.failed_count();
        }
        total
    }

    pub async fn panics_caught_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.panics_caught_count();
        }
        total
    }

    pub async fn memory_usage_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for shard in self.shards.iter() {
            total += shard.lock().await.memory_usage_bytes();
        }
        total
    }

    pub async fn reset_counts(&self) {
        for shard in self.shards.iter() {
            shard.lock().await.reset_counts();
        }
    }
}
