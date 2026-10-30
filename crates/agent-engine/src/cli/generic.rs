use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Instant;
use tokio::process::Command;
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
                rendered = rendered.replace("{workspace}", &input.working_dir);
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

        let mut cmd = Command::new(&self.binary_path);
        cmd.args(&rendered_args);

        // 2. Set working directory
        if !input.working_dir.is_empty() {
            cmd.current_dir(&input.working_dir);
        }

        // 3. Set custom environment variables
        for (k, v) in &self.env_vars {
            cmd.env(k, v);
        }

        // 4. Execute with streaming or buffered timeout
        let timeout_duration = if input.timeout_seconds > 0 {
            std::time::Duration::from_secs(input.timeout_seconds)
        } else {
            std::time::Duration::from_secs(300)
        };

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            EngineError::ProcessFailed(format!(
                "Failed to spawn generic CLI [{}]: {}",
                self.name, e
            ))
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let cb_opt = input.on_stream_chunk.clone();

        let run_future = async {
            let mut accumulated_stdout = String::new();
            if let Some(out) = stdout {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(ref cb) = cb_opt {
                        cb(&line);
                        cb("\n");
                    }
                    accumulated_stdout.push_str(&line);
                    accumulated_stdout.push('\n');
                }
            }

            let mut accumulated_stderr = String::new();
            if let Some(err) = stderr {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    accumulated_stderr.push_str(&line);
                    accumulated_stderr.push('\n');
                }
            }

            let status = child.wait().await.map_err(|e| {
                EngineError::ProcessFailed(format!(
                    "Failed to wait for generic CLI [{}]: {}",
                    self.name, e
                ))
            })?;

            Ok::<_, EngineError>((status, accumulated_stdout, accumulated_stderr))
        };

        let result = match tokio::time::timeout(timeout_duration, run_future).await {
            Ok(res) => res?,
            Err(_) => return Err(EngineError::Timeout),
        };

        let (status, stdout, stderr) = result;
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
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Generic(self.name.clone())
    }
}
