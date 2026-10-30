use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;

pub struct MockEngine {
    pub response_template: String,
}

impl MockEngine {
    pub fn new(template: &str) -> Self {
        Self {
            response_template: template.to_string(),
        }
    }
}

#[async_trait]
impl ComputeEngine for MockEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        Ok(EngineOutput {
            stdout_content: format!("{}: {}", self.response_template, input.prompt_or_command),
            exit_code: 0,
            execution_duration_ms: 10,
            engine_used: EngineKind::Mock,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}
