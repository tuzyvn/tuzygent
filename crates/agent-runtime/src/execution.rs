pub mod benchmark;
pub mod fault;
pub mod resilience;

pub use benchmark::{ExecutionBenchmarkHarness, ExecutionBenchmarkStats};
pub use fault::{
    is_retryable_execution_error, simulate_execution_fault, ExecutionFault, ExecutionFaultPolicy,
};
pub use resilience::ResilientExecutionService;

use crate::sandbox::{SandboxConfig, SandboxEnvironment};
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tracing::info;

struct CatchUnwindFuture<F>(F);
impl<F: Future> Future for CatchUnwindFuture<F> {
    type Output = std::thread::Result<F::Output>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let f = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.poll(cx))) {
            Ok(Poll::Ready(val)) => Poll::Ready(Ok(val)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

/// Unified Execution Service that guarantees every engine execution
/// adheres to sandboxing, guardrails, timeout, fault injection, retry and output limits.
#[derive(Clone)]
pub struct ExecutionService {
    workspace_dir: PathBuf,
    sandbox_config: SandboxConfig,
    guardrail: GuardrailPolicy,
    cancellation_token: Option<Arc<AtomicBool>>,
    fault_policy: Arc<Mutex<ExecutionFaultPolicy>>,
    max_retries: usize,
}

impl ExecutionService {
    pub fn from_workspace(workspace_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let config = agent_engine::EnginesConfig::load_hierarchical(Some(workspace_dir.as_ref()));
        let sandbox = SandboxConfig::from_engines_config(&config)?;
        Ok(Self::new(workspace_dir, Some(sandbox), None))
    }

    pub fn new(
        workspace_dir: impl AsRef<Path>,
        sandbox_config: Option<SandboxConfig>,
        guardrail: Option<GuardrailPolicy>,
    ) -> Self {
        let s_config = sandbox_config.unwrap_or_default();
        let g_policy = guardrail.unwrap_or_else(GuardrailPolicy::default_enterprise);
        Self {
            workspace_dir: workspace_dir.as_ref().to_path_buf(),
            sandbox_config: s_config,
            guardrail: g_policy,
            cancellation_token: None,
            fault_policy: Arc::new(Mutex::new(ExecutionFaultPolicy::none())),
            max_retries: 0,
        }
    }

    pub fn with_guardrail(mut self, guardrail: GuardrailPolicy) -> Self { self.guardrail = guardrail; self }
    pub fn with_sandbox_config(mut self, sandbox_config: SandboxConfig) -> Self { self.sandbox_config = sandbox_config; self }
    pub fn with_workspace_dir(mut self, workspace_dir: impl AsRef<Path>) -> Self { self.workspace_dir = workspace_dir.as_ref().to_path_buf(); self }
    pub fn with_cancellation_token(mut self, token: Arc<AtomicBool>) -> Self { self.cancellation_token = Some(token); self }
    pub fn with_max_retries(mut self, retries: usize) -> Self { self.max_retries = retries; self }
    pub fn with_fault_policy(self, policy: ExecutionFaultPolicy) -> Self {
        if let Ok(mut guard) = self.fault_policy.lock() { *guard = policy; }
        self
    }

    pub fn workspace_dir(&self) -> &Path { &self.workspace_dir }
    pub fn guardrail(&self) -> &GuardrailPolicy { &self.guardrail }
    pub fn sandbox_config(&self) -> &SandboxConfig { &self.sandbox_config }
    pub fn cancellation_token(&self) -> Option<&Arc<AtomicBool>> { self.cancellation_token.as_ref() }
    pub fn max_retries(&self) -> usize { self.max_retries }

    pub fn fault_policy(&self) -> ExecutionFaultPolicy {
        match self.fault_policy.lock() {
            Ok(p) => p.clone(),
            Err(_) => ExecutionFaultPolicy::default(),
        }
    }

    pub async fn execute(
        &self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        self.execute_cancellable(engine, input, None).await
    }

    pub async fn execute_resilient(
        &self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let mut resilient = self.clone();
        if resilient.max_retries == 0 { resilient.max_retries = 3; }
        resilient.execute(engine, input).await
    }

    pub async fn execute_cancellable(
        &self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
        token: Option<&Arc<AtomicBool>>,
    ) -> Result<EngineOutput, EngineError> {
        let effective_cancel = token.or(self.cancellation_token.as_ref());
        if let Some(cancel) = effective_cancel {
            if cancel.load(Ordering::SeqCst) {
                return Err(EngineError::ProcessFailed("Execution canceled before start".into()));
            }
        }

        if !input.prompt_or_command.is_empty() {
            if let Err(err) = self.guardrail.check_command_safety(&input.prompt_or_command) {
                return Err(EngineError::PolicyRejected(err));
            }
        }

        let mut s_input = input.clone();
        if s_input.working_dir.is_empty() {
            s_input.working_dir = self.workspace_dir.to_string_lossy().to_string();
        }
        let max_bytes = match s_input.max_output_bytes {
            Some(b) => b.min(self.guardrail.max_output_bytes),
            None => self.guardrail.max_output_bytes,
        };
        s_input.max_output_bytes = Some(max_bytes);

        let timeout_secs = if s_input.timeout_seconds == 0 { 300 } else { s_input.timeout_seconds };
        s_input.timeout_seconds = timeout_secs;

        let workdir = Path::new(&s_input.working_dir);
        let s_env = SandboxEnvironment::with_config(&self.workspace_dir, self.sandbox_config.clone());
        s_input.command_sandbox = s_env.command_sandbox(workdir);

        info!("🛡️ [ExecutionService] Executing '{}' (timeout: {}s, retries: {})",
            engine.engine_kind(), timeout_secs, self.max_retries);

        let mut attempt = 0;
        let timeout_duration = std::time::Duration::from_secs(timeout_secs);
        loop {
            attempt += 1;
            if let Some(cancel) = effective_cancel {
                if cancel.load(Ordering::SeqCst) {
                    return Err(EngineError::ProcessFailed("Execution canceled during run".into()));
                }
            }

            let injected_fault = match self.fault_policy.lock() {
                Ok(mut guard) => guard.should_inject(),
                Err(_) => None,
            };
            if let Some(fault) = injected_fault {
                let retryable = fault.is_transient() || fault == ExecutionFault::Panic;
                if retryable && attempt <= self.max_retries {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    continue;
                }
                return Err(fault.to_engine_error());
            }

            let eng = engine.clone();
            let inp = s_input.clone();
            let fut_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| eng.execute(&inp)));
            let fut = match fut_res {
                Ok(f) => f,
                Err(_) => {
                    if attempt <= self.max_retries {
                        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                        continue;
                    }
                    return Err(EngineError::ProcessFailed("Execution worker panicked unexpectedly".into()));
                }
            };
            let guarded_fut = CatchUnwindFuture(fut);

            let outcome: Result<EngineOutput, EngineError> = if let Some(cancel) = effective_cancel {
                let c = cancel.clone();
                tokio::select! {
                    biased;
                    _ = async {
                        while !c.load(Ordering::SeqCst) {
                            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        }
                    } => {
                        Err(EngineError::ProcessFailed("Execution canceled during run".into()))
                    }
                    res = tokio::time::timeout(timeout_duration, guarded_fut) => match res {
                        Err(_) => Err(EngineError::Timeout),
                        Ok(Err(_)) => Err(EngineError::ProcessFailed("Execution worker panicked unexpectedly".into())),
                        Ok(Ok(eo)) => eo,
                    }
                }
            } else {
                match tokio::time::timeout(timeout_duration, guarded_fut).await {
                    Err(_) => Err(EngineError::Timeout),
                    Ok(Err(_)) => Err(EngineError::ProcessFailed("Execution worker panicked unexpectedly".into())),
                    Ok(Ok(eo)) => eo,
                }
            };

            match outcome {
                Ok(mut out) => {
                    if out.stdout_content.len() > max_bytes {
                        let mut end = max_bytes;
                        while end > 0 && !out.stdout_content.is_char_boundary(end) { end -= 1; }
                        let mut trunc = out.stdout_content[..end].to_string();
                        trunc.push_str("\n[... output truncated by ExecutionService guardrail ...]");
                        out.stdout_content = trunc;
                    }
                    return Ok(out);
                }
                Err(e) => {
                    if is_retryable_execution_error(&e) && attempt <= self.max_retries {
                        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

