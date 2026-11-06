use crate::execution::fault::{is_retryable_execution_error, ExecutionFault, ExecutionFaultPolicy};
use crate::execution::ExecutionService;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Resilient execution service with automatic retry, fault injection testing, panic isolation,
/// bounded memory caching, and audit ledger recording
#[derive(Clone)]
pub struct ResilientExecutionService {
    service: ExecutionService,
    policy: ExecutionFaultPolicy,
    max_retries: usize,
    total_executed: usize,
    recovered_count: usize,
    failed_count: usize,
    panics_caught_count: usize,
    response_cache: Arc<Mutex<HashMap<String, EngineOutput>>>,
    cache_queue: Arc<Mutex<VecDeque<String>>>,
    max_capacity: Option<usize>,
}

impl ResilientExecutionService {
    pub fn new(service: ExecutionService, policy: ExecutionFaultPolicy, max_retries: usize) -> Self {
        Self {
            service,
            policy,
            max_retries,
            total_executed: 0,
            recovered_count: 0,
            failed_count: 0,
            panics_caught_count: 0,
            response_cache: Arc::new(Mutex::new(HashMap::new())),
            cache_queue: Arc::new(Mutex::new(VecDeque::new())),
            max_capacity: None,
        }
    }

    pub fn from_workspace(
        workspace_dir: impl AsRef<Path>,
        policy: ExecutionFaultPolicy,
        max_retries: usize,
    ) -> anyhow::Result<Self> {
        let service = ExecutionService::from_workspace(workspace_dir)?;
        Ok(Self::new(service, policy, max_retries))
    }

    pub fn service(&self) -> &ExecutionService {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut ExecutionService {
        &mut self.service
    }

    pub fn policy(&self) -> &ExecutionFaultPolicy {
        &self.policy
    }

    pub fn set_policy(&mut self, policy: ExecutionFaultPolicy) {
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

    pub fn panics_caught_count(&self) -> usize {
        self.panics_caught_count
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    pub fn with_capacity(self, capacity: usize) -> Self {
        if let Ok(mut cache) = self.response_cache.lock() {
            cache.reserve(capacity);
        }
        if let Ok(mut queue) = self.cache_queue.lock() {
            queue.reserve(capacity);
        }
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.evict_if_needed();
        self
    }

    fn evict_if_needed(&self) {
        if let Some(max) = self.max_capacity {
            if let (Ok(mut cache), Ok(mut queue)) =
                (self.response_cache.lock(), self.cache_queue.lock())
            {
                while queue.len() >= max && max > 0 {
                    if let Some(old_key) = queue.pop_front() {
                        cache.remove(&old_key);
                    }
                }
            }
        }
    }

    pub fn cache_response(&self, key: &str, output: &EngineOutput) {
        if let Some(max) = self.max_capacity {
            if max == 0 {
                return;
            }
            if let (Ok(mut cache), Ok(mut queue)) =
                (self.response_cache.lock(), self.cache_queue.lock())
            {
                while queue.len() >= max {
                    if let Some(old_key) = queue.pop_front() {
                        cache.remove(&old_key);
                    }
                }
                if !cache.contains_key(key) {
                    queue.push_back(key.to_string());
                }
                cache.insert(key.to_string(), output.clone());
            }
        }
    }

    pub fn get_cached(&self, key: &str) -> Option<EngineOutput> {
        self.response_cache.lock().ok()?.get(key).cloned()
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let (cache_overhead, heap_content) = if let Ok(cache) = self.response_cache.lock() {
            let overhead = cache.capacity()
                * (std::mem::size_of::<String>() + std::mem::size_of::<EngineOutput>());
            let content: usize = cache
                .iter()
                .map(|(k, v)| k.len() + v.stdout_content.len() + std::mem::size_of::<EngineOutput>())
                .sum();
            (overhead, content)
        } else {
            (0, 0)
        };
        let queue_overhead = if let Ok(queue) = self.cache_queue.lock() {
            queue.capacity() * std::mem::size_of::<String>()
        } else {
            0
        };
        base_size + cache_overhead + queue_overhead + heap_content
    }

    pub async fn execute_batch(
        &mut self,
        engine: Arc<dyn ComputeEngine>,
        inputs: &[EngineInput],
    ) -> Vec<Result<EngineOutput, EngineError>> {
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results.push(self.execute_resilient(Arc::clone(&engine), input).await);
        }
        results
    }

    /// Execute the compute engine with resilience, retrying on transient errors and isolating panics
    pub async fn execute_resilient(
        &mut self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        // Red line policy violations are non-retryable and must fail immediately
        if !input.prompt_or_command.is_empty() {
            if let Err(red_line_err) = self.service.guardrail().check_command_safety(&input.prompt_or_command) {
                self.failed_count += 1;
                return Err(EngineError::PolicyRejected(red_line_err));
            }
        }

        let mut attempt = 0;
        loop {
            attempt += 1;

            // Check if a simulated chaos fault should be injected
            if let Some(fault) = self.policy.should_inject() {
                if fault == ExecutionFault::Panic {
                    self.panics_caught_count += 1;
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }
                    self.failed_count += 1;
                    return Err(fault.to_engine_error());
                }

                if attempt <= self.max_retries {
                    self.recovered_count += 1;
                    continue;
                }

                self.failed_count += 1;
                return Err(fault.to_engine_error());
            }

            // Spawn execution in task boundary to isolate potential panics from the worker
            let srv = self.service.clone();
            let eng = engine.clone();
            let inp = input.clone();
            let task_result = tokio::spawn(async move {
                srv.execute(eng, &inp).await
            }).await;

            match task_result {
                Ok(Ok(output)) => {
                    self.total_executed += 1;
                    self.cache_response(&input.prompt_or_command, &output);
                    return Ok(output);
                }
                Ok(Err(engine_err)) => {
                    if let EngineError::ProcessFailed(ref msg) = engine_err {
                        if msg.contains("panicked") {
                            self.panics_caught_count += 1;
                        }
                    }

                    if !is_retryable_execution_error(&engine_err) {
                        self.failed_count += 1;
                        return Err(engine_err);
                    }

                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }

                    self.failed_count += 1;
                    return Err(engine_err);
                }
                Err(join_err) => {
                    if join_err.is_panic() {
                        self.panics_caught_count += 1;
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        }
                        self.failed_count += 1;
                        return Err(EngineError::ProcessFailed(
                            "Execution worker panicked unexpectedly".to_string(),
                        ));
                    }

                    self.failed_count += 1;
                    return Err(EngineError::ProcessFailed(format!(
                        "Execution task failed: {}",
                        join_err
                    )));
                }
            }
        }
    }

    /// Resilient execution with integrated audit ledger recording
    pub async fn execute_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let command_summary = if input.prompt_or_command.len() > 60 {
            format!("{}...", &input.prompt_or_command[..60])
        } else {
            input.prompt_or_command.clone()
        };

        let result = self.execute_resilient(engine, input).await;
        match &result {
            Ok(output) => {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!(
                            "ExecutionService command executed successfully [exit_code: {}]: {}",
                            output.exit_code, command_summary
                        ),
                    );
                }));
            }
            Err(err) => {
                let (event_type, severity) = match err {
                    EngineError::PolicyRejected(_) => (
                        AuditEventType::CommandBlocked,
                        ActionSeverity::Destructive,
                    ),
                    _ => (
                        AuditEventType::CommandException,
                        ActionSeverity::HighImpact,
                    ),
                };
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ledger.record_event(
                        event_type,
                        severity,
                        format!(
                            "ExecutionService failed for command '{}': {}",
                            command_summary, err
                        ),
                    );
                }));
            }
        }

        result
    }
}
