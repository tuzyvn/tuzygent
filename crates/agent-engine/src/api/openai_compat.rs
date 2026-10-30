use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use serde_json::json;
use std::time::Instant;

pub struct OpenAiCompatibleEngine {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleEngine {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ComputeEngine for OpenAiCompatibleEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let start = Instant::now();
        let url = format!("{}/chat/completions", self.base_url);

        let messages = vec![
            json!({
                "role": "system",
                "content": input.system_context
            }),
            json!({
                "role": "user",
                "content": input.prompt_or_command
            }),
        ];

        let req_body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0.8,
            "max_tokens": 500
        });

        let res = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| EngineError::ApiRequestFailed(format!("HTTP POST error to LLM: {}", e)))?;

        let status = res.status().as_u16() as i32;
        if status != 200 {
            let err_text = res.text().await.unwrap_or_default();
            return Err(EngineError::ApiRequestFailed(format!(
                "LLM API returned status {}: {}",
                status, err_text
            )));
        }

        let resp_json: serde_json::Value = res.json().await.map_err(|e| {
            EngineError::ApiRequestFailed(format!("Failed parsing LLM JSON: {}", e))
        })?;

        let content = resp_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        let duration = start.elapsed().as_millis();

        Ok(EngineOutput {
            stdout_content: content,
            exit_code: 0,
            execution_duration_ms: duration,
            engine_used: EngineKind::CustomAgentApi,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::CustomAgentApi
    }
}
