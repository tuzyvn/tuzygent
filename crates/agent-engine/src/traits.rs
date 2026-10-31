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
    Generic(String),
}

impl std::fmt::Display for EngineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineKind::ClaudeCode => write!(f, "claude"),
            EngineKind::Antigravity => write!(f, "antigravity"),
            EngineKind::Codex => write!(f, "codex"),
            EngineKind::CustomAgentApi => write!(f, "custom_api"),
            EngineKind::Mock => write!(f, "mock"),
            EngineKind::Generic(name) => write!(f, "generic:{}", name),
        }
    }
}

impl EngineKind {
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "claude" | "claude_code" | "claudecode" => EngineKind::ClaudeCode,
            "antigravity" | "agy" => EngineKind::Antigravity,
            "codex" => EngineKind::Codex,
            "custom" | "custom_api" => EngineKind::CustomAgentApi,
            "mock" => EngineKind::Mock,
            other => EngineKind::Generic(other.to_string()),
        }
    }
}

pub type EngineEventCallback = std::sync::Arc<dyn Fn(&serde_json::Value) + Send + Sync>;

pub type StreamCallback = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone, Serialize, Deserialize)]
pub struct EngineInput {
    #[serde(skip)]
    pub on_event: Option<EngineEventCallback>,
    #[serde(skip)]
    pub command_sandbox: Option<crate::process::CommandSandbox>,
    pub prompt_or_command: String,
    pub system_context: String,
    pub working_dir: String,
    pub timeout_seconds: u64,
    #[serde(skip)]
    pub on_stream_chunk: Option<StreamCallback>,
}

impl std::fmt::Debug for EngineInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineInput")
            .field("prompt_or_command", &self.prompt_or_command)
            .field("system_context", &self.system_context)
            .field("working_dir", &self.working_dir)
            .field("timeout_seconds", &self.timeout_seconds)
            .field("has_stream_callback", &self.on_stream_chunk.is_some())
            .finish()
    }
}

impl EngineInput {
    pub fn new(prompt: impl Into<String>, working_dir: impl Into<String>) -> Self {
        Self {
            prompt_or_command: prompt.into(),
            system_context: String::new(),
            working_dir: working_dir.into(),
            timeout_seconds: 300,
            on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
        }
    }

    pub fn with_stream_callback(mut self, cb: StreamCallback) -> Self {
        self.on_stream_chunk = Some(cb);
        self
    }
}

impl Default for EngineInput {
    fn default() -> Self {
        Self {
            prompt_or_command: String::new(),
            system_context: String::new(),
            working_dir: ".".to_string(),
            timeout_seconds: 300,
            on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
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
