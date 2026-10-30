use async_trait::async_trait;
use serde_json::json;
use std::time::Instant;
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

pub struct CustomAgentApiEngine {
    pub endpoint_url: String,
    pub api_key: Option<String>,
    client: reqwest::Client,
}

impl CustomAgentApiEngine {
    pub fn new(endpoint_url: &str, api_key: Option<String>) -> Self {
        Self {
            endpoint_url: endpoint_url.to_string(),
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ComputeEngine for CustomAgentApiEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let start = Instant::now();

        let mut req = self.client.post(&self.endpoint_url).json(&json!({
            "prompt": input.prompt_or_command,
            "system": input.system_context,
            "working_dir": input.working_dir
        }));

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let res = req.send().await.map_err(|e| {
            EngineError::ApiRequestFailed(format!("HTTP request error: {}", e))
        })?;

        let status = res.status().as_u16() as i32;
        let body = res.text().await.map_err(|e| {
            EngineError::ApiRequestFailed(format!("Failed reading response: {}", e))
        })?;

        let duration = start.elapsed().as_millis();

        Ok(EngineOutput {
            stdout_content: body,
            exit_code: if status == 200 { 0 } else { status },
            execution_duration_ms: duration,
            engine_used: EngineKind::CustomAgentApi,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::CustomAgentApi
    }
}
