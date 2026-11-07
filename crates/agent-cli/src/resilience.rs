use crate::cli::Cli;
use crate::fault::{is_retryable_cli_error, CliExecutionError, CliFault, CliFaultPolicy};
use agent_domain::{ActionSeverity, AuditLedger};
use std::collections::VecDeque;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Resilient CLI Runner with automatic retry, panic isolation, audit ledger tracing, and fault injection
#[derive(Clone)]
pub struct ResilientCliRunner {
    policy: CliFaultPolicy,
    max_retries: usize,
    total_executed: usize,
    recovered_count: usize,
    failed_count: usize,
    panics_caught_count: usize,
    audit_workspace: Option<PathBuf>,
    ledger: AuditLedger,
    max_capacity: Option<usize>,
    recent_actions: VecDeque<String>,
}

impl Default for ResilientCliRunner {
    fn default() -> Self {
        Self::new(CliFaultPolicy::none(), 3)
    }
}

impl ResilientCliRunner {
    pub fn new(policy: CliFaultPolicy, max_retries: usize) -> Self {
        Self {
            policy,
            max_retries,
            total_executed: 0,
            recovered_count: 0,
            failed_count: 0,
            panics_caught_count: 0,
            audit_workspace: None,
            ledger: AuditLedger::new(),
            max_capacity: None,
            recent_actions: VecDeque::new(),
        }
    }

    pub fn with_audit_workspace(mut self, path: impl AsRef<Path>) -> Self {
        self.audit_workspace = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_policy(mut self, policy: CliFaultPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_max_retries(mut self, max: usize) -> Self {
        self.max_retries = max;
        self
    }

    pub fn with_max_capacity(mut self, max: usize) -> Self {
        self.max_capacity = Some(max);
        self.recent_actions.reserve(max.min(1024));
        self.evict_if_needed();
        self
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    pub fn policy(&self) -> &CliFaultPolicy {
        &self.policy
    }

    pub fn set_policy(&mut self, policy: CliFaultPolicy) {
        self.policy = policy;
    }

    pub fn total_executed(&self) -> usize {
        self.total_executed
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn failed_count(&self) -> usize {
        self.failed_count
    }

    pub fn rejected_count(&self) -> usize {
        self.failed_count
    }

    pub fn panics_caught_count(&self) -> usize {
        self.panics_caught_count
    }

    pub fn ledger(&self) -> &AuditLedger {
        &self.ledger
    }

    pub fn recent_actions(&self) -> &VecDeque<String> {
        &self.recent_actions
    }

    pub fn is_bounded(&self) -> bool {
        self.max_capacity.is_some()
    }

    pub fn reset_counts(&mut self) {
        self.total_executed = 0;
        self.recovered_count = 0;
        self.failed_count = 0;
        self.panics_caught_count = 0;
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let ledger_overhead = self.ledger.len() * std::mem::size_of::<agent_domain::AuditEntry>();
        let heap_content: usize = self
            .ledger
            .entries()
            .iter()
            .map(|e| e.action.len() + e.details.len() + e.prev_hash.len() + e.hash.len())
            .sum();
        let queue_overhead = self.recent_actions.capacity() * (std::mem::size_of::<String>() + 32);
        base_size + ledger_overhead + heap_content + queue_overhead
    }

    fn evict_if_needed(&mut self) {
        if let Some(max) = self.max_capacity {
            if max == 0 {
                self.ledger = AuditLedger::new();
                self.recent_actions.clear();
                return;
            }
            if self.ledger.len() > max {
                let excess = self.ledger.len() - max;
                let retained = self.ledger.entries()[excess..].to_vec();
                self.ledger = AuditLedger::from_entries(retained);
            }
            while self.recent_actions.len() > max {
                let _ = self.recent_actions.pop_front();
            }
        }
    }

    fn record_audit(&mut self, action: &str, severity: ActionSeverity, details: &str) {
        self.recent_actions.push_back(action.to_string());
        self.ledger.record(action, severity, details);
        self.evict_if_needed();
        if let Some(ref ws) = self.audit_workspace {
            let audit_file = ws.join(".audit_ledger.json");
            if let Ok(json_str) = serde_json::to_string_pretty(self.ledger.entries()) {
                let _ = std::fs::write(&audit_file, json_str);
            }
        }
    }

    fn check_fault_injection(
        &mut self,
        act: &str,
        att: usize,
    ) -> Option<Result<(), CliExecutionError>> {
        let fault = self.policy.should_inject()?;
        if fault == CliFault::Panic {
            self.panics_caught_count += 1;
            return if att <= self.max_retries {
                self.recovered_count += 1;
                self.record_audit(
                    act,
                    ActionSeverity::HighImpact,
                    "Simulated CLI panic isolated; retrying",
                );
                Some(Ok(()))
            } else {
                self.failed_count += 1;
                Some(Err(CliExecutionError::Panic(
                    "Simulated panic max retries exceeded".to_string(),
                )))
            };
        }
        let simulated_err = fault.to_execution_error();
        if is_retryable_cli_error(&simulated_err) && att <= self.max_retries {
            self.recovered_count += 1;
            let msg = format!(
                "Injected transient fault {:?} recovered on retry attempt {}",
                fault, att
            );
            self.record_audit(act, ActionSeverity::Routine, &msg);
            Some(Ok(()))
        } else {
            self.failed_count += 1;
            Some(Err(simulated_err))
        }
    }

    fn handle_error_outcome(
        &mut self,
        act: &str,
        att: usize,
        err: CliExecutionError,
    ) -> Result<(), CliExecutionError> {
        if is_retryable_cli_error(&err) && att <= self.max_retries {
            self.recovered_count += 1;
            let msg = format!(
                "Retryable error '{}' encountered; retrying attempt {}/{}",
                err, att, self.max_retries
            );
            self.record_audit(act, ActionSeverity::HighImpact, &msg);
            Ok(())
        } else {
            self.failed_count += 1;
            let msg = format!("CLI action '{}' failed permanently: {}", act, err);
            self.record_audit(act, ActionSeverity::Destructive, &msg);
            Err(err)
        }
    }

    fn handle_panic_outcome(
        &mut self,
        act: &str,
        att: usize,
        payload: Box<dyn std::any::Any + Send>,
    ) -> Result<(), CliExecutionError> {
        self.panics_caught_count += 1;
        let panic_msg = if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };
        if att <= self.max_retries {
            self.recovered_count += 1;
            let msg = format!(
                "Critical panic isolated in CLI runner: {}; retrying {}/{}",
                panic_msg, att, self.max_retries
            );
            self.record_audit(act, ActionSeverity::Destructive, &msg);
            Ok(())
        } else {
            self.failed_count += 1;
            Err(CliExecutionError::Panic(panic_msg))
        }
    }

    pub async fn run_resilient<F, Fut, T>(
        &mut self,
        action_name: &str,
        mut op: F,
    ) -> Result<T, CliExecutionError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, CliExecutionError>>,
    {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(check) = self.check_fault_injection(action_name, attempt) {
                match check {
                    Ok(()) => continue,
                    Err(err) => return Err(err),
                }
            }
            let exec_fut = AssertUnwindSafe(op());
            match futures::FutureExt::catch_unwind(exec_fut).await {
                Ok(Ok(val)) => {
                    self.total_executed += 1;
                    let msg = format!("CLI action '{}' completed successfully", action_name);
                    self.record_audit(action_name, ActionSeverity::Routine, &msg);
                    return Ok(val);
                }
                Ok(Err(err)) => {
                    self.handle_error_outcome(action_name, attempt, err)?;
                }
                Err(payload) => {
                    self.handle_panic_outcome(action_name, attempt, payload)?;
                }
            }
        }
    }

    pub fn run_resilient_sync<F, T>(
        &mut self,
        action_name: &str,
        mut op: F,
    ) -> Result<T, CliExecutionError>
    where
        F: FnMut() -> Result<T, CliExecutionError>,
    {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(check) = self.check_fault_injection(action_name, attempt) {
                match check {
                    Ok(()) => continue,
                    Err(err) => return Err(err),
                }
            }
            match std::panic::catch_unwind(AssertUnwindSafe(&mut op)) {
                Ok(Ok(val)) => {
                    self.total_executed += 1;
                    let msg = format!("CLI action '{}' completed successfully (sync)", action_name);
                    self.record_audit(action_name, ActionSeverity::Routine, &msg);
                    return Ok(val);
                }
                Ok(Err(err)) => {
                    self.handle_error_outcome(action_name, attempt, err)?;
                }
                Err(payload) => {
                    self.handle_panic_outcome(action_name, attempt, payload)?;
                }
            }
        }
    }

    pub async fn run_command_resilient(&mut self, cli: Cli) -> Result<(), CliExecutionError> {
        self.run_resilient("cli_command_dispatch", || {
            let cli_clone = cli.clone();
            async move {
                crate::commands::run_command(cli_clone)
                    .await
                    .map_err(CliExecutionError::from)
            }
        })
        .await
    }

    pub async fn execute_batch_operations<T: Clone>(
        &mut self,
        operations: &[(&str, Result<T, CliExecutionError>)],
    ) -> Vec<Result<T, CliExecutionError>> {
        let mut results = Vec::with_capacity(operations.len());
        for (action_name, outcome) in operations {
            let res = self
                .run_resilient(action_name, || {
                    let outcome_clone = outcome.clone();
                    async move { outcome_clone }
                })
                .await;
            results.push(res);
        }
        results
    }

    pub async fn run_batch_operations<T: Clone>(
        &mut self,
        operations: &[(&str, Result<T, CliExecutionError>)],
    ) -> Vec<Result<T, CliExecutionError>> {
        self.execute_batch_operations(operations).await
    }

    pub async fn execute_batch_commands(
        &mut self,
        commands: &[Cli],
    ) -> Vec<Result<(), CliExecutionError>> {
        let mut results = Vec::with_capacity(commands.len());
        for cmd in commands {
            results.push(self.run_command_resilient(cmd.clone()).await);
        }
        results
    }
}

/// Sharded Resilient CLI Runner distributing executions across N shards to eliminate lock contention
#[derive(Clone)]
pub struct ShardedResilientCliRunner {
    shards: Arc<Vec<tokio::sync::Mutex<ResilientCliRunner>>>,
    num_shards: usize,
    total_capacity: Option<usize>,
}

impl ShardedResilientCliRunner {
    pub fn new(policy: CliFaultPolicy, max_retries: usize, num_shards: usize) -> Self {
        let n = if num_shards == 0 { 16 } else { num_shards };
        let shards = (0..n)
            .map(|_| tokio::sync::Mutex::new(ResilientCliRunner::new(policy.clone(), max_retries)))
            .collect();
        Self {
            shards: Arc::new(shards),
            num_shards: n,
            total_capacity: None,
        }
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.total_capacity = Some(max_capacity);
        let per_shard = (max_capacity / self.num_shards).max(1) + 1;
        if let Some(shards) = Arc::get_mut(&mut self.shards) {
            for shard in shards.iter_mut() {
                let runner = shard.get_mut();
                *runner = runner.clone().with_max_capacity(per_shard);
            }
        }
        self
    }

    pub fn with_audit_workspace(mut self, path: impl AsRef<Path>) -> Self {
        let p = path.as_ref();
        if let Some(shards) = Arc::get_mut(&mut self.shards) {
            for shard in shards.iter_mut() {
                let runner = shard.get_mut();
                *runner = runner.clone().with_audit_workspace(p);
            }
        }
        self
    }

    pub fn shard_idx(&self, key: &str) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn shard(&self, idx: usize) -> &tokio::sync::Mutex<ResilientCliRunner> {
        &self.shards[idx % self.num_shards]
    }

    pub fn shard_for(&self, key: &str) -> &tokio::sync::Mutex<ResilientCliRunner> {
        self.shard(self.shard_idx(key))
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    pub fn total_capacity(&self) -> Option<usize> {
        self.total_capacity
    }

    pub async fn run_resilient<F, Fut, T>(
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
        guard.run_resilient(action_name, op).await
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
