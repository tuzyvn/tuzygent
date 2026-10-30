use async_trait::async_trait;
use tokio::process::Command;
use std::time::Instant;
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

pub struct ClaudeCodeEngine {
    pub binary_path: String,
    pub model: Option<String>,
}

impl ClaudeCodeEngine {
    pub fn new(binary_path: Option<String>, model: Option<String>) -> Self {
        Self {
            binary_path: binary_path.unwrap_or_else(|| "claude".to_string()),
            model,
        }
    }
}

#[async_trait]
impl ComputeEngine for ClaudeCodeEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let start = Instant::now();

        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("-p")
            .arg(&input.prompt_or_command);

        if let Some(m) = &self.model {
            cmd.arg("--model").arg(m);
        }

        if !input.working_dir.is_empty() {
            cmd.current_dir(&input.working_dir);
        }

        let output = cmd.output()
            .await
            .map_err(|e| EngineError::ProcessFailed(format!("Failed to run claude CLI: {}", e)))?;

        let duration = start.elapsed().as_millis();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(EngineOutput {
            stdout_content: stdout,
            exit_code: output.status.code().unwrap_or(-1),
            execution_duration_ms: duration,
            engine_used: EngineKind::ClaudeCode,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::ClaudeCode
    }
}
