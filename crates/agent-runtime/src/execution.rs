use crate::sandbox::{SandboxConfig, SandboxEnvironment};
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

/// Unified Execution Service that guarantees every engine execution
/// adheres to sandboxing, guardrails, timeout, and output limit policies.
#[derive(Clone)]
pub struct ExecutionService {
    workspace_dir: PathBuf,
    sandbox_config: SandboxConfig,
    guardrail: GuardrailPolicy,
}

impl ExecutionService {
    pub fn new(
        workspace_dir: impl AsRef<Path>,
        sandbox_config: Option<SandboxConfig>,
        guardrail: Option<GuardrailPolicy>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.as_ref().to_path_buf(),
            sandbox_config: sandbox_config.unwrap_or_default(),
            guardrail: guardrail.unwrap_or_else(GuardrailPolicy::default_enterprise),
        }
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

    /// Execute the provided compute engine through mandatory sandbox and policy gates
    pub async fn execute(
        &self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
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

        if sandboxed_input.max_output_bytes.is_none() {
            sandboxed_input.max_output_bytes = Some(self.guardrail.max_output_bytes);
        }

        // 3. Inject sandbox environment
        let workdir_path = Path::new(&sandboxed_input.working_dir);
        let sandbox_env = SandboxEnvironment::with_config(
            &self.workspace_dir,
            self.sandbox_config.clone(),
        );
        sandboxed_input.command_sandbox = sandbox_env.command_sandbox(workdir_path);

        info!(
            "🛡️ [ExecutionService] Executing engine '{}' with sandbox isolation",
            engine.engine_kind()
        );

        // 4. Delegate to engine
        engine.execute(&sandboxed_input).await
    }
}
