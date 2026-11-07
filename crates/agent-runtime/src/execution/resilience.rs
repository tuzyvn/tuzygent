use crate::execution::{ExecutionFaultPolicy, ExecutionService};
use agent_engine::EngineOutput;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

mod audit;
mod cache;
mod execute;

/// Resilient execution service with retries, panic isolation, bounded caching, and audit recording.
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
    backoff_policy: Option<agent_engine::BackoffPolicy>,
}

impl ResilientExecutionService {
    pub fn new(
        service: ExecutionService,
        policy: ExecutionFaultPolicy,
        max_retries: usize,
    ) -> Self {
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
            backoff_policy: None,
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

    pub fn with_backoff_policy(mut self, policy: agent_engine::BackoffPolicy) -> Self {
        self.backoff_policy = Some(policy.clone());
        self.service = self.service.with_backoff_policy(policy);
        self
    }

    pub fn backoff_policy(&self) -> Option<&agent_engine::BackoffPolicy> {
        self.backoff_policy.as_ref()
    }
}
