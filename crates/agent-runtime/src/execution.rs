use crate::sandbox::{SandboxConfig, SandboxEnvironment};
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::info;

/// Unified Execution Service that guarantees every engine execution
/// adheres to sandboxing, guardrails, timeout, and output limit policies.
#[derive(Clone)]
pub struct ExecutionService {
    workspace_dir: PathBuf,
    sandbox_config: SandboxConfig,
    guardrail: GuardrailPolicy,
    cancellation_token: Option<Arc<AtomicBool>>,
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
        Self {
            workspace_dir: workspace_dir.as_ref().to_path_buf(),
            sandbox_config: sandbox_config.unwrap_or_default(),
            guardrail: guardrail.unwrap_or_else(GuardrailPolicy::default_enterprise),
            cancellation_token: None,
        }
    }

    pub fn with_guardrail(mut self, guardrail: GuardrailPolicy) -> Self {
        self.guardrail = guardrail;
        self
    }

    pub fn with_sandbox_config(mut self, sandbox_config: SandboxConfig) -> Self {
        self.sandbox_config = sandbox_config;
        self
    }

    pub fn with_workspace_dir(mut self, workspace_dir: impl AsRef<Path>) -> Self {
        self.workspace_dir = workspace_dir.as_ref().to_path_buf();
        self
    }

    pub fn with_cancellation_token(mut self, token: Arc<AtomicBool>) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    pub fn guardrail(&self) -> &GuardrailPolicy {
        &self.guardrail
    }

    pub fn sandbox_config(&self) -> &SandboxConfig {
        &self.sandbox_config
    }

    pub fn cancellation_token(&self) -> Option<&Arc<AtomicBool>> {
        self.cancellation_token.as_ref()
    }

    /// Execute the provided compute engine through mandatory sandbox, policy, timeout and output limits
    pub async fn execute(
        &self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        self.execute_cancellable(engine, input, None).await
    }

    /// Execute with optional ad-hoc cancellation token
    pub async fn execute_cancellable(
        &self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
        token: Option<&Arc<AtomicBool>>,
    ) -> Result<EngineOutput, EngineError> {
        let effective_cancel = token.or(self.cancellation_token.as_ref());
        if let Some(cancel) = effective_cancel {
            if cancel.load(Ordering::SeqCst) {
                return Err(EngineError::ProcessFailed(
                    "Execution canceled before start".to_string(),
                ));
            }
        }

        // 1. Check command safety against guardrails
        if !input.prompt_or_command.is_empty() {
            if let Err(red_line_err) = self.guardrail.check_command_safety(&input.prompt_or_command) {
                return Err(EngineError::PolicyRejected(red_line_err));
            }
        }

        // 2. Prepare Sandboxed Input
        let mut sandboxed_input = input.clone();
        if sandboxed_input.working_dir.is_empty() {
            sandboxed_input.working_dir = self.workspace_dir.to_string_lossy().to_string();
        }

        let max_output_bytes = sandboxed_input
            .max_output_bytes
            .unwrap_or(self.guardrail.max_output_bytes)
            .min(self.guardrail.max_output_bytes);
        sandboxed_input.max_output_bytes = Some(max_output_bytes);

        // Safe fallback for zero timeout to prevent immediate timeout error
        let timeout_secs = if sandboxed_input.timeout_seconds == 0 {
            300
        } else {
            sandboxed_input.timeout_seconds
        };
        sandboxed_input.timeout_seconds = timeout_secs;

        // 3. Inject sandbox environment
        let workdir_path = Path::new(&sandboxed_input.working_dir);
        let sandbox_env = SandboxEnvironment::with_config(
            &self.workspace_dir,
            self.sandbox_config.clone(),
        );
        sandboxed_input.command_sandbox = sandbox_env.command_sandbox(workdir_path);

        info!(
            "🛡️ [ExecutionService] Executing engine '{}' with sandbox isolation (timeout: {}s, max_bytes: {})",
            engine.engine_kind(),
            timeout_secs,
            max_output_bytes,
        );

        // 4. Delegate to engine with timeout and cancellation support
        let engine_fut = engine.execute(&sandboxed_input);
        let timeout_duration = std::time::Duration::from_secs(timeout_secs);

        let mut output = if let Some(cancel) = effective_cancel {
            let cancel_clone = cancel.clone();
            tokio::select! {
                biased;
                _ = async {
                    while !cancel_clone.load(Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                } => {
                    return Err(EngineError::ProcessFailed("Execution canceled during run".to_string()));
                }
                res = tokio::time::timeout(timeout_duration, engine_fut) => {
                    res.map_err(|_| EngineError::Timeout)??
                }
            }
        } else {
            tokio::time::timeout(timeout_duration, engine_fut)
                .await
                .map_err(|_| EngineError::Timeout)??
        };

        // 5. Enforce output limit and truncate safely at UTF-8 char boundary
        if output.stdout_content.len() > max_output_bytes {
            let mut end = max_output_bytes;
            while end > 0 && !output.stdout_content.is_char_boundary(end) {
                end -= 1;
            }
            let mut truncated = output.stdout_content[..end].to_string();
            truncated.push_str("\n[... output truncated by ExecutionService guardrail ...]");
            output.stdout_content = truncated;
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_engine::{EngineKind, EngineOutput};
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct RecordingEngine(Mutex<Option<EngineInput>>);

    #[async_trait]
    impl ComputeEngine for RecordingEngine {
        async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
            let mut guard = self
                .0
                .lock()
                .map_err(|e| EngineError::ProcessFailed(format!("Lock error: {e}")))?;
            *guard = Some(input.clone());
            Ok(EngineOutput {
                stdout_content: "ok".into(),
                exit_code: 0,
                execution_duration_ms: 0,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }

    #[tokio::test]
    async fn execution_applies_sandbox_and_output_limit() -> Result<(), Box<dyn std::error::Error>> {
        let engine = Arc::new(RecordingEngine(Mutex::new(None)));
        let service = ExecutionService::new("/tmp", None, None);
        service
            .execute(engine.clone(), &EngineInput::new("hello", "/tmp"))
            .await?;
        let guard = engine
            .0
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        let input = guard.as_ref().ok_or("Recorded input is missing")?;
        assert!(input.command_sandbox.is_some());
        assert_eq!(
            input.max_output_bytes,
            Some(service.guardrail().max_output_bytes)
        );
        Ok(())
    }

    #[tokio::test]
    async fn policy_rejects_before_engine_is_called() -> Result<(), Box<dyn std::error::Error>> {
        let engine = Arc::new(RecordingEngine(Mutex::new(None)));
        let service = ExecutionService::new("/tmp", None, None);
        assert!(matches!(
            service
                .execute(engine.clone(), &EngineInput::new("rm -rf /", "/tmp"))
                .await,
            Err(EngineError::PolicyRejected(_))
        ));
        let guard = engine
            .0
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        assert!(guard.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn recording_engine_handles_poisoned_mutex_gracefully() {
        let engine = Arc::new(RecordingEngine(Mutex::new(None)));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = engine.0.lock();
            std::panic::resume_unwind(Box::new("intentional poison"));
        }));
        let result = engine.execute(&EngineInput::new("test", "/tmp")).await;
        assert!(matches!(result, Err(EngineError::ProcessFailed(_))));
    }
}
