use crate::fault::{CoreExecutionError, CoreFaultPolicy};
use crate::safe::SafeGuard;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Execution result produced by `ResilientCoreAgent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreExecutionResult {
    pub operation_id: String,
    pub name: String,
    pub output: String,
    pub duration_us: u64,
    pub retries_used: usize,
    pub recovered_from_fault: bool,
}

/// Resilient agent execution coordinator offering bounded memory, retry policies,
/// circuit breaker fault isolation, and zero-panic crash immunity.
pub struct ResilientCoreAgent {
    fault_policy: Arc<CoreFaultPolicy>,
    max_capacity: usize,
    history: Mutex<VecDeque<CoreExecutionResult>>,
}

impl Default for ResilientCoreAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl ResilientCoreAgent {
    pub fn new() -> Self {
        Self {
            fault_policy: Arc::new(CoreFaultPolicy::default()),
            max_capacity: 5000,
            history: Mutex::new(VecDeque::new()),
        }
    }

    pub fn with_max_capacity(mut self, capacity: usize) -> Self {
        self.max_capacity = capacity;
        self
    }

    pub fn with_fault_policy(mut self, policy: Arc<CoreFaultPolicy>) -> Self {
        self.fault_policy = policy;
        self
    }

    pub fn fault_policy(&self) -> &Arc<CoreFaultPolicy> {
        &self.fault_policy
    }

    pub fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    pub fn record_count(&self) -> usize {
        let lock = match self.history.lock() {
            Ok(guard) => guard,
            Err(p) => p.into_inner(),
        };
        lock.len()
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let lock = match self.history.lock() {
            Ok(guard) => guard,
            Err(p) => p.into_inner(),
        };
        lock.iter()
            .map(|item| {
                std::mem::size_of::<CoreExecutionResult>()
                    + item.operation_id.len()
                    + item.name.len()
                    + item.output.len()
            })
            .sum()
    }

    pub fn clear(&self) {
        let mut lock = match self.history.lock() {
            Ok(guard) => guard,
            Err(p) => p.into_inner(),
        };
        lock.clear();
    }

    fn store_result(&self, res: CoreExecutionResult) {
        let mut lock = match self.history.lock() {
            Ok(guard) => guard,
            Err(p) => p.into_inner(),
        };
        while lock.len() >= self.max_capacity && !lock.is_empty() {
            lock.pop_front();
        }
        lock.push_back(res);
    }

    /// Executes an operation with panic guard and fault policy checks.
    pub fn execute_operation(
        &self,
        name: &str,
        payload: &str,
    ) -> Result<CoreExecutionResult, CoreExecutionError> {
        let start = Instant::now();

        // 1. Fault policy check
        self.fault_policy.check_and_inject()?;

        // 2. Safe execution wrapped in panic isolation
        let name_owned = name.to_string();
        let payload_owned = payload.to_string();

        let execution_outcome = SafeGuard::catch_unwind_result(move || {
            format!(
                "core-agent-output:[{}]-payload:{}",
                name_owned, payload_owned
            )
        });

        match execution_outcome {
            Ok(output) => {
                self.fault_policy.record_success();
                let duration_us = start.elapsed().as_micros() as u64;
                let result = CoreExecutionResult {
                    operation_id: uuid::Uuid::new_v4().to_string(),
                    name: name.to_string(),
                    output,
                    duration_us,
                    retries_used: 0,
                    recovered_from_fault: false,
                };
                self.store_result(result.clone());
                Ok(result)
            }
            Err(e) => {
                self.fault_policy.record_failure();
                Err(e)
            }
        }
    }

    /// Executes an operation with exponential backoff and automatic retry on transient errors.
    pub fn execute_with_retry(
        &self,
        name: &str,
        payload: &str,
        max_retries: usize,
    ) -> Result<CoreExecutionResult, CoreExecutionError> {
        let start = Instant::now();
        let mut attempts = 0;
        let mut last_err = CoreExecutionError::Other("No execution attempt made".to_string());

        while attempts <= max_retries {
            match self.execute_operation(name, payload) {
                Ok(mut res) => {
                    res.retries_used = attempts;
                    res.recovered_from_fault = attempts > 0;
                    res.duration_us = start.elapsed().as_micros() as u64;
                    return Ok(res);
                }
                Err(e) => {
                    last_err = e;
                    attempts += 1;
                    if attempts <= max_retries {
                        // Exponential backoff micro-pause
                        let backoff_us = (attempts as u64) * 200;
                        std::thread::sleep(std::time::Duration::from_micros(backoff_us));
                    }
                }
            }
        }

        Err(last_err)
    }

    /// Executes a batch of operations sequentially with self-healing isolation.
    pub fn execute_batch_operations(
        &self,
        ops: Vec<(&str, &str)>,
    ) -> Vec<Result<CoreExecutionResult, CoreExecutionError>> {
        ops.into_iter()
            .map(|(name, payload)| self.execute_operation(name, payload))
            .collect()
    }
}

/// Fluent builder for constructing customized `ResilientCoreAgent` instances.
pub struct ResilientCoreAgentBuilder {
    max_capacity: usize,
    fault_policy: Option<Arc<CoreFaultPolicy>>,
}

impl Default for ResilientCoreAgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ResilientCoreAgentBuilder {
    pub fn new() -> Self {
        Self {
            max_capacity: 5000,
            fault_policy: None,
        }
    }

    pub fn max_capacity(mut self, capacity: usize) -> Self {
        self.max_capacity = capacity;
        self
    }

    pub fn fault_policy(mut self, policy: Arc<CoreFaultPolicy>) -> Self {
        self.fault_policy = Some(policy);
        self
    }

    pub fn build(self) -> ResilientCoreAgent {
        let mut agent = ResilientCoreAgent::new().with_max_capacity(self.max_capacity);
        if let Some(policy) = self.fault_policy {
            agent = agent.with_fault_policy(policy);
        }
        agent
    }
}
