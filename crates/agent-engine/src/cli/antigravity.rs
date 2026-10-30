use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityUsage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub thinking_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityStepUpdate {
    #[serde(default)]
    pub conversation_id: String,
    pub step_index: Option<i32>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub step_type: String,
    #[serde(default)]
    pub text_delta: String,
    pub usage: Option<AntigravityUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityResultEvent {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub error: String,
    pub usage: Option<AntigravityUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityStreamEvent {
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub conversation_id: String,
    pub step_update: Option<AntigravityStepUpdate>,
    pub result: Option<AntigravityResultEvent>,
}

pub struct AntigravityEngine {
    pub binary_path: String,
    pub model: Option<String>,
}

fn resolve_agy_binary(binary_path: Option<String>) -> String {
    if let Some(p) = binary_path {
        if !p.trim().is_empty() {
            return p;
        }
    }
    if let Ok(env_bin) = std::env::var("ANTIGRAVITY_BIN").or_else(|_| std::env::var("AGY_BIN")) {
        if !env_bin.trim().is_empty() {
            return env_bin;
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if !home.is_empty() {
        let candidate = std::path::Path::new(&home)
            .join(".local")
            .join("bin")
            .join("agy");
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }
    "agy".to_string()
}

impl AntigravityEngine {
    pub fn new(binary_path: Option<String>, model: Option<String>) -> Self {
        Self {
            binary_path: resolve_agy_binary(binary_path),
            model,
        }
    }
}

#[async_trait]
impl ComputeEngine for AntigravityEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let start = Instant::now();

        // Ghép instruction và prompt
        let full_prompt = if !input.system_context.is_empty() {
            format!(
                "[INSTRUCTION]\n{}\n\n[USER INPUT]\n{}",
                input.system_context, input.prompt_or_command
            )
        } else {
            input.prompt_or_command.clone()
        };

        // Tham khảo Multica: cấu hình stream-json và output-format
        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("-p")
            .arg(&full_prompt)
            .arg("--output-format")
            .arg("stream-json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(m) = &self.model {
            cmd.arg("--model").arg(m);
        }

        if !input.working_dir.is_empty() {
            cmd.current_dir(&input.working_dir);
        }

        let mut child = cmd.spawn().map_err(|e| {
            EngineError::ProcessFailed(format!("Failed to spawn Antigravity CLI (agy): {}", e))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            EngineError::ProcessFailed("Failed to capture stdout of agy".to_string())
        })?;

        let mut reader = BufReader::new(stdout).lines();
        let mut accumulated_text = String::new();
        let mut final_response = String::new();
        let mut total_usage = AntigravityUsage {
            input_tokens: 0,
            output_tokens: 0,
            thinking_tokens: 0,
            total_tokens: 0,
        };

        // Stream NDJSON parsing loop (Multica stream-scanner pattern)
        while let Ok(Some(line)) = reader.next_line().await {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(event) = serde_json::from_str::<AntigravityStreamEvent>(&line) {
                match event.event.as_str() {
                    "step_update" => {
                        if let Some(step) = event.step_update {
                            if !step.text_delta.is_empty() {
                                if let Some(ref cb) = input.on_stream_chunk {
                                    cb(&step.text_delta);
                                }
                                accumulated_text.push_str(&step.text_delta);
                                debug!("agy text delta: {}", step.text_delta);
                            }
                            if let Some(u) = step.usage {
                                total_usage.input_tokens = u.input_tokens;
                                total_usage.output_tokens = u.output_tokens;
                                total_usage.thinking_tokens = u.thinking_tokens;
                                total_usage.total_tokens = u.total_tokens;
                            }
                        }
                    }
                    "result" => {
                        if let Some(res) = event.result {
                            if !res.response.is_empty() {
                                final_response = res.response;
                            }
                            if let Some(u) = res.usage {
                                total_usage = u;
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                // Fallback nếu agy trả text thô (plain text lines)
                if !accumulated_text.is_empty() {
                    accumulated_text.push('\n');
                }
                accumulated_text.push_str(&line);
            }
        }

        let status = child.wait().await.map_err(|e| {
            EngineError::ProcessFailed(format!("Failed to wait for agy child process: {}", e))
        })?;

        let duration = start.elapsed().as_millis();

        // Theo Multica: Ưu tiên canonical result.response, nếu rỗng thì dùng accumulated stream text
        let mut final_output = if !final_response.is_empty() {
            final_response
        } else {
            accumulated_text
        };

        // Xử lý regression trống output trong một số phiên bản của agy
        if final_output.trim().is_empty() && status.success() {
            warn!("⚠️ [Antigravity] Emitted empty output on success, applying fallback notice");
            final_output = "[Antigravity Completed Execution with Zero Direct Output]".to_string();
        }

        if total_usage.total_tokens > 0 {
            info!(
                "📊 [Antigravity Usage] In: {}, Out: {}, Think: {}, Total: {}",
                total_usage.input_tokens,
                total_usage.output_tokens,
                total_usage.thinking_tokens,
                total_usage.total_tokens
            );
        }

        Ok(EngineOutput {
            stdout_content: final_output.trim().to_string(),
            exit_code: status.code().unwrap_or(-1),
            execution_duration_ms: duration,
            engine_used: EngineKind::Antigravity,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Antigravity
    }
}
