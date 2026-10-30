use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Instant;
use tokio::process::Command;
use tracing::{debug, info};

pub struct ClaudeCodeEngine {
    pub binary_path: String,
    pub model: Option<String>,
}

fn resolve_claude_binary(binary_path: Option<String>) -> String {
    if let Some(p) = binary_path {
        if !p.trim().is_empty() {
            return p;
        }
    }
    if let Ok(env_bin) = std::env::var("CLAUDE_BIN") {
        if !env_bin.trim().is_empty() {
            return env_bin;
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if !home.is_empty() {
        let candidates = [
            std::path::Path::new(&home)
                .join(".local")
                .join("bin")
                .join("claude"),
            std::path::Path::new(&home)
                .join(".npm-global")
                .join("bin")
                .join("claude"),
        ];
        for candidate in &candidates {
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    "claude".to_string()
}

impl ClaudeCodeEngine {
    pub fn new(binary_path: Option<String>, model: Option<String>) -> Self {
        Self {
            binary_path: resolve_claude_binary(binary_path),
            model,
        }
    }
}

#[async_trait]
impl ComputeEngine for ClaudeCodeEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let start = Instant::now();

        let prompt = if !input.system_context.is_empty() {
            format!("{}\n\n{}", input.system_context, input.prompt_or_command)
        } else {
            input.prompt_or_command.clone()
        };

        info!("Spawning Claude Code CLI at {}", self.binary_path);

        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("-p").arg(&prompt);
        cmd.arg("--output-format").arg("json");

        if let Some(m) = &self.model {
            cmd.arg("--model").arg(m);
        }

        if !input.working_dir.is_empty() {
            cmd.current_dir(&input.working_dir);
        }

        let timeout_duration = if input.timeout_seconds > 0 {
            std::time::Duration::from_secs(input.timeout_seconds)
        } else {
            std::time::Duration::from_secs(300)
        };

        let output_res = tokio::time::timeout(timeout_duration, cmd.output()).await;

        let output = match output_res {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(EngineError::ProcessFailed(format!(
                    "Failed to run claude CLI at {}: {}",
                    self.binary_path, e
                )));
            }
            Err(_) => return Err(EngineError::Timeout),
        };

        let duration = start.elapsed().as_millis();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        // Extract clean text from JSON output if available
        let final_text = if let Ok(json) = serde_json::from_str::<Value>(&stdout) {
            if let Some(result_text) = json.get("result").and_then(|r| r.as_str()) {
                result_text.to_string()
            } else if let Some(content) = json.get("content").and_then(|c| c.as_str()) {
                content.to_string()
            } else {
                stdout
            }
        } else {
            stdout
        };

        debug!("Claude Code finished in {}ms", duration);

        Ok(EngineOutput {
            stdout_content: final_text,
            exit_code: output.status.code().unwrap_or(-1),
            execution_duration_ms: duration,
            engine_used: EngineKind::ClaudeCode,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::ClaudeCode
    }
}
