use crate::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Non-interactive Codex CLI adapter. Every execute starts a new session;
/// resuming requires an explicit session UUID, never a shared `--last` session.
pub struct CodexEngine {
    pub binary_path: String,
    pub model: Option<String>,
    pub env_vars: HashMap<String, String>,
}

#[derive(Default)]
struct RunState {
    session_id: Option<String>,
    final_message: String,
    completed: bool,
    failure: Option<String>,
    diagnostic: Option<String>,
    invalid_json: bool,
}
impl RunState {
    fn accept(&mut self, event: &Value) -> Option<String> {
        match event["type"].as_str() {
            Some("thread.started") => {
                self.session_id = event["thread_id"].as_str().map(str::to_owned);
            }
            Some("item.completed") if event["item"]["type"] == "agent_message" => {
                let item = &event["item"];
                if item["phase"].as_str() != Some("commentary") {
                    if let Some(text) = item["text"].as_str() {
                        self.final_message = text.to_owned();
                        return Some(text.to_owned());
                    }
                }
            }
            Some("turn.completed") => self.completed = true,
            Some("turn.failed") => {
                self.failure = Some(
                    event["error"]["message"]
                        .as_str()
                        .unwrap_or("Codex turn failed")
                        .to_owned(),
                );
            }
            Some("error") => {
                // Connection retries may emit an error before a successful turn.
                self.diagnostic = event["message"].as_str().map(str::to_owned);
            }
            _ => {}
        }
        None
    }
}

impl CodexEngine {
    pub fn new(binary_path: Option<String>, model: Option<String>) -> Self {
        let binary_path = binary_path
            .filter(|p| !p.trim().is_empty())
            .or_else(|| {
                std::env::var("CODEX_BIN")
                    .ok()
                    .filter(|p| !p.trim().is_empty())
            })
            .unwrap_or_else(|| "codex".into());
        Self {
            binary_path,
            model,
            env_vars: HashMap::new(),
        }
    }
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env_vars = env;
        self
    }
    pub async fn resume(
        &self,
        input: &EngineInput,
        session_id: &str,
    ) -> Result<EngineOutput, EngineError> {
        uuid::Uuid::parse_str(session_id).map_err(|_| {
            EngineError::ProcessFailed("Codex resume requires an explicit session UUID".into())
        })?;
        self.run(input, Some(session_id)).await
    }
    async fn run(
        &self,
        input: &EngineInput,
        session_id: Option<&str>,
    ) -> Result<EngineOutput, EngineError> {
        let start = Instant::now();
        let prompt = if input.system_context.is_empty() {
            input.prompt_or_command.clone()
        } else {
            format!(
                "[INSTRUCTIONS]\n{}\n\n[TASK]\n{}",
                input.system_context, input.prompt_or_command
            )
        };
        let mut args = vec![
            "exec".into(),
            "--json".into(),
            "--sandbox".into(),
            "workspace-write".into(),
            "--config".into(),
            "approval_policy=\"never\"".into(),
        ];
        if let Some(model) = &self.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(id) = session_id {
            args.extend(["resume".into(), id.into()]);
        }
        // A task beginning with a dash must remain data, not become a CLI option.
        args.extend(["--".into(), prompt]);
        let state = Arc::new(Mutex::new(RunState::default()));
        let received = state.clone();
        let stream = input.on_stream_chunk.clone();
        let events = input.on_event.clone();
        let callback = Arc::new(move |line: &str| {
            let event: Value = match serde_json::from_str(line) {
                Ok(event) => event,
                Err(_) => {
                    received.lock().unwrap().invalid_json = true;
                    return;
                }
            };
            let text = received.lock().unwrap().accept(&event);
            if let Some(callback) = &events {
                callback(&event);
            }
            if let (Some(callback), Some(text)) = (&stream, text) {
                callback(&text);
                callback("\n");
            }
        });
        let mut input = input.clone();
        if let Some(sandbox) = &mut input.command_sandbox {
            if sandbox.program == "bwrap" {
                if let Some(directory) = std::path::Path::new(&self.binary_path).parent() {
                    for companion in [
                        directory.join("codex-exec-server"),
                        directory.join("codex-code-mode-host"),
                    ] {
                        if companion.is_absolute() && companion.is_file() {
                            sandbox.prefix_args.extend([
                                "--ro-bind".into(),
                                companion.to_string_lossy().into_owned(),
                                companion.to_string_lossy().into_owned(),
                            ]);
                        }
                    }
                }
            }
        }
        let output = crate::process::run_command(
            &input,
            &self.binary_path,
            &args,
            &self.env_vars,
            Some(callback),
        )
        .await?;
        let mut state = state.lock().unwrap();
        let mut exit_code = output.status.code().unwrap_or(-1);
        let mut failure = state.failure.take();
        if exit_code == 0 {
            if state.invalid_json {
                failure = Some("Codex emitted invalid JSONL output".into());
            } else if !state.completed {
                failure = failure
                    .or_else(|| state.diagnostic.clone())
                    .or_else(|| Some("Codex did not emit turn.completed".into()));
            } else if state.final_message.trim().is_empty() {
                failure = Some("Codex completed without a final agent message".into());
            }
            if failure.is_some() {
                exit_code = 1;
            }
        }
        let content = if exit_code != 0 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = failure
                .or_else(|| state.diagnostic.clone())
                .unwrap_or_else(|| format!("Codex CLI exited with code {exit_code}"));
            if stderr.trim().is_empty() {
                reason
            } else {
                format!("{reason}\n{}", stderr.trim())
            }
        } else {
            state.final_message.clone()
        };
        Ok(EngineOutput {
            stdout_content: content,
            exit_code,
            execution_duration_ms: start.elapsed().as_millis(),
            engine_used: EngineKind::Codex,
            session_id: state.session_id.clone(),
        })
    }
}
#[async_trait]
impl ComputeEngine for CodexEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        self.run(input, None).await
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Codex
    }
}
