use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
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
    pub env_vars: HashMap<String, String>,
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
            env_vars: HashMap::new(),
        }
    }

    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env_vars = env;
        self
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

        let mut args = vec![
            "-p".into(),
            full_prompt,
            "--output-format".into(),
            "stream-json".into(),
            "--mode".into(),
            "auto".into(),
            "--dangerously-skip-permissions".into(),
        ];
        if let Some(model) = &self.model {
            args.extend(["--model".into(), model.clone()]);
        }
        let callback = input.on_stream_chunk.clone().map(|cb| {
            std::sync::Arc::new(move |line: &str| {
                if let Ok(event) = serde_json::from_str::<AntigravityStreamEvent>(line) {
                    if let Some(step) = event.step_update {
                        if !step.text_delta.is_empty() {
                            cb(&step.text_delta);
                        }
                    }
                }
            }) as crate::StreamCallback
        });
        let output =
            crate::process::run_command(input, &self.binary_path, &args, &self.env_vars, callback)
                .await?;
        let mut accumulated_text = String::new();
        let mut final_response = String::new();
        let mut total_usage = AntigravityUsage {
            input_tokens: 0,
            output_tokens: 0,
            thinking_tokens: 0,
            total_tokens: 0,
        };

        // Stream NDJSON parsing loop (Multica stream-scanner pattern)
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(event) = serde_json::from_str::<AntigravityStreamEvent>(line) {
                match event.event.as_str() {
                    "step_update" => {
                        if let Some(step) = event.step_update {
                            if !step.text_delta.is_empty() {
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
                accumulated_text.push_str(line);
            }
        }

        let status = output.status;

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
            session_id: None,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Antigravity
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AntigravityResultEvent, AntigravityStepUpdate, AntigravityStreamEvent, AntigravityUsage,
    };
    use serde::Serialize;
    use serde_json::{json, Value};

    fn assert_json_round_trip<T>(wire_json: Value)
    where
        T: serde::de::DeserializeOwned + Serialize,
    {
        let decoded: T = serde_json::from_value(wire_json.clone()).expect("decode wire JSON");
        let encoded = serde_json::to_value(decoded).expect("encode wire JSON");
        assert_eq!(encoded, wire_json);
    }

    #[test]
    fn usage_serialization_matches_wire_fields_and_defaults_missing_counters(
    ) -> Result<(), serde_json::Error> {
        assert_json_round_trip::<AntigravityUsage>(json!({
            "input_tokens": 12,
            "output_tokens": 8,
            "thinking_tokens": 3,
            "total_tokens": 23
        }));

        let usage: AntigravityUsage = serde_json::from_value(json!({}))?;
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.thinking_tokens, 0);
        assert_eq!(usage.total_tokens, 0);

        Ok(())
    }

    #[test]
    fn usage_with_invalid_counter_returns_deserialization_error() {
        let result = serde_json::from_value::<AntigravityUsage>(json!({
            "input_tokens": "not-a-number"
        }));

        assert!(
            result.is_err(),
            "invalid counter should be reported as an error"
        );
    }

    #[test]
    fn step_update_serialization_matches_wire_fields() {
        assert_json_round_trip::<AntigravityStepUpdate>(json!({
            "conversation_id": "conversation-1",
            "step_index": 2,
            "state": "running",
            "step_type": "assistant",
            "text_delta": "hello",
            "usage": {
                "input_tokens": 12,
                "output_tokens": 8,
                "thinking_tokens": 3,
                "total_tokens": 23
            }
        }));
    }

    #[test]
    fn result_event_serialization_matches_wire_fields() {
        assert_json_round_trip::<AntigravityResultEvent>(json!({
            "conversation_id": "conversation-1",
            "status": "completed",
            "response": "final answer",
            "error": "",
            "usage": {
                "input_tokens": 12,
                "output_tokens": 8,
                "thinking_tokens": 3,
                "total_tokens": 23
            }
        }));
    }

    #[test]
    fn stream_event_serialization_matches_nested_wire_fields() {
        assert_json_round_trip::<AntigravityStreamEvent>(json!({
            "event": "step_update",
            "conversation_id": "conversation-1",
            "step_update": {
                "conversation_id": "conversation-1",
                "step_index": 2,
                "state": "running",
                "step_type": "assistant",
                "text_delta": "hello",
                "usage": null
            },
            "result": null
        }));
    }
}
