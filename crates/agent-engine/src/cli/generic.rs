use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct GenericCliEngine {
    pub name: String,
    pub binary_path: String,
    pub args_template: Vec<String>,
    pub env_vars: HashMap<String, String>,
    pub output_parser: Option<String>,
}

impl GenericCliEngine {
    pub fn new(
        name: impl Into<String>,
        binary_path: impl Into<String>,
        args_template: Vec<String>,
        env_vars: HashMap<String, String>,
        output_parser: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            binary_path: binary_path.into(),
            args_template,
            env_vars,
            output_parser,
        }
    }
}

#[async_trait]
impl ComputeEngine for GenericCliEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let start = Instant::now();

        // 1. Prepare command arguments
        let mut rendered_args = Vec::new();
        let mut prompt_substituted = false;

        for arg in &self.args_template {
            let mut rendered = arg.clone();
            if rendered.contains("{prompt}") {
                rendered = rendered.replace("{prompt}", &input.prompt_or_command);
                prompt_substituted = true;
            }
            if rendered.contains("{system_context}") {
                rendered = rendered.replace("{system_context}", &input.system_context);
            }
            if rendered.contains("{workspace}") {
                rendered = rendered.replace(
                    "{workspace}",
                    input
                        .command_sandbox
                        .as_ref()
                        .map(|s| s.working_dir.as_str())
                        .unwrap_or(&input.working_dir),
                );
            }
            rendered_args.push(rendered);
        }

        // If template did not explicitly include {prompt}, append it
        if !prompt_substituted && !input.prompt_or_command.is_empty() {
            rendered_args.push(input.prompt_or_command.clone());
        }

        info!(
            "🚀 Spawning Generic CLI [{}] -> {} {:?}",
            self.name, self.binary_path, rendered_args
        );

        let output = crate::process::run_command(
            input,
            &self.binary_path,
            &rendered_args,
            &self.env_vars,
            input.on_stream_chunk.clone(),
        )
        .await?;
        let status = output.status;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let duration = start.elapsed().as_millis();

        let final_content = if status.success() || !stdout.is_empty() {
            stdout
        } else {
            format!("Error output:\n{}", stderr)
        };

        debug!("Generic CLI [{}] finished in {}ms", self.name, duration);

        Ok(EngineOutput {
            stdout_content: final_content,
            exit_code: status.code().unwrap_or(-1),
            execution_duration_ms: duration,
            engine_used: EngineKind::Generic(self.name.clone()),
            session_id: None,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Generic(self.name.clone())
    }
}
