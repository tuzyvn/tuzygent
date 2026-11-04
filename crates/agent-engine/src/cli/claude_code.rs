use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, info};

pub struct ClaudeCodeEngine {
    pub binary_path: String,
    pub model: Option<String>,
    pub env_vars: HashMap<String, String>,
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
            env_vars: HashMap::new(),
        }
    }

    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env_vars = env;
        self
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

        let mut args = vec!["-p".into(), prompt, "--output-format".into(), "json".into()];
        if let Some(model) = &self.model {
            args.extend(["--model".into(), model.clone()]);
        }
        let output =
            crate::process::run_command(input, &self.binary_path, &args, &self.env_vars, None)
                .await?;

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
            session_id: None,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::ClaudeCode
    }
}
