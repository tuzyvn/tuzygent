use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Process execution error: {0}")]
    ProcessFailed(String),
    #[error("CLI engine not found or unreachable: {0}")]
    CliNotFound(String),
    #[error("API request failed: {0}")]
    ApiRequestFailed(String),
    #[error("Timeout exceeded during computation")]
    Timeout,
    #[error("Red line policy rejected execution: {0}")]
    PolicyRejected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EngineKind {
    ClaudeCode,
    Antigravity,
    Codex,
    CustomAgentApi,
    Mock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInput {
    pub prompt_or_command: String,
    pub system_context: String,
    pub working_dir: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineOutput {
    pub stdout_content: String,
    pub exit_code: i32,
    pub execution_duration_ms: u128,
    pub engine_used: EngineKind,
}

#[async_trait]
pub trait ComputeEngine: Send + Sync {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError>;
    fn engine_kind(&self) -> EngineKind;
}
