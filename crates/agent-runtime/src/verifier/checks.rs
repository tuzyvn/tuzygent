use agent_domain::{VerificationResult, VerificationSpec};
use tracing::{debug, info};

use super::{format::take_tail_chars, TaskVerifier};

impl TaskVerifier<'_> {
    /// Executes verification for a single specification
    pub async fn verify_spec(&self, spec: &VerificationSpec) -> VerificationResult {
        let now = chrono::Utc::now();
        match spec {
            VerificationSpec::FileExists { path, min_bytes } => {
                let full_path = self.workspace_dir.join(path);
                if !full_path.exists() {
                    return VerificationResult {
                        passed: false,
                        spec: spec.clone(),
                        details: format!("Target file does not exist: {:?}", path),
                        executed_at: now,
                    };
                }
                if let Some(min_b) = min_bytes {
                    match tokio::fs::metadata(&full_path).await {
                        Ok(meta) if meta.len() < *min_b => {
                            return VerificationResult {
                                passed: false,
                                spec: spec.clone(),
                                details: format!("File {:?} size ({} bytes) is less than required minimum ({} bytes)", path, meta.len(), min_b),
                                executed_at: now,
                            };
                        }
                        Err(e) => {
                            return VerificationResult {
                                passed: false,
                                spec: spec.clone(),
                                details: format!("Failed to read metadata for {:?}: {}", path, e),
                                executed_at: now,
                            };
                        }
                        _ => {}
                    }
                }
                VerificationResult {
                    passed: true,
                    spec: spec.clone(),
                    details: format!("Verified file exists and is valid: {:?}", path),
                    executed_at: now,
                }
            }
            VerificationSpec::JsonValid { path } => {
                let full_path = self.workspace_dir.join(path);
                match tokio::fs::read_to_string(&full_path).await {
                    Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(_) => VerificationResult {
                            passed: true,
                            spec: spec.clone(),
                            details: format!("File {:?} is valid JSON", path),
                            executed_at: now,
                        },
                        Err(e) => VerificationResult {
                            passed: false,
                            spec: spec.clone(),
                            details: format!("File {:?} is not valid JSON: {}", path, e),
                            executed_at: now,
                        },
                    },
                    Err(e) => VerificationResult {
                        passed: false,
                        spec: spec.clone(),
                        details: format!("Failed to read JSON file {:?}: {}", path, e),
                        executed_at: now,
                    },
                }
            }
            VerificationSpec::ContentContains { path, pattern } => {
                let full_path = self.workspace_dir.join(path);
                match tokio::fs::read_to_string(&full_path).await {
                    Ok(content) => {
                        if content.contains(pattern) {
                            VerificationResult {
                                passed: true,
                                spec: spec.clone(),
                                details: format!(
                                    "File {:?} contains required pattern '{}'",
                                    path, pattern
                                ),
                                executed_at: now,
                            }
                        } else {
                            VerificationResult {
                                passed: false,
                                spec: spec.clone(),
                                details: format!(
                                    "File {:?} does not contain required pattern '{}'",
                                    path, pattern
                                ),
                                executed_at: now,
                            }
                        }
                    }
                    Err(e) => VerificationResult {
                        passed: false,
                        spec: spec.clone(),
                        details: format!("Failed to read file {:?}: {}", path, e),
                        executed_at: now,
                    },
                }
            }
            VerificationSpec::Command {
                command,
                expected_exit_code,
            } => {
                info!("Executing verification command: {}", command);
                if !self.guardrail.allow_shell_execution
                    || self.guardrail.check_command_safety(command).is_err()
                {
                    return VerificationResult {
                        passed: false,
                        spec: spec.clone(),
                        details: "Verification command rejected by policy".into(),
                        executed_at: now,
                    };
                }
                let mut input = agent_engine::EngineInput::new(
                    command,
                    self.workspace_dir.to_string_lossy().as_ref(),
                );
                input.timeout_seconds = 60;
                input.max_output_bytes = Some(self.guardrail.max_output_bytes);
                input.command_sandbox = crate::sandbox::SandboxEnvironment::with_config(
                    self.workspace_dir,
                    self.sandbox.clone(),
                )
                .command_sandbox(self.workspace_dir);
                match agent_engine::process::run_command(
                    &input,
                    "sh",
                    &["-c".into(), command.clone()],
                    &std::collections::HashMap::new(),
                    None,
                )
                .await
                {
                    Ok(output) => {
                        let actual_code = output.status.code().unwrap_or(-1);
                        let passed = actual_code == *expected_exit_code;
                        let stdout_lossy = String::from_utf8_lossy(&output.stdout);
                        let stderr_lossy = String::from_utf8_lossy(&output.stderr);
                        let stdout_tail = take_tail_chars(&stdout_lossy, 500);
                        let stderr_tail = take_tail_chars(&stderr_lossy, 500);
                        VerificationResult {
                            passed,
                            spec: spec.clone(),
                            details: format!(
                                "Exit code: {} (expected {}). stdout: {} | stderr: {}",
                                actual_code,
                                expected_exit_code,
                                stdout_tail.trim(),
                                stderr_tail.trim()
                            ),
                            executed_at: now,
                        }
                    }
                    Err(e) => VerificationResult {
                        passed: false,
                        spec: spec.clone(),
                        details: format!("Verification command '{}' failed: {}", command, e),
                        executed_at: now,
                    },
                }
            }
            VerificationSpec::Custom { description } => {
                debug!("Custom verification note: {}", description);
                VerificationResult {
                    passed: false,
                    spec: spec.clone(),
                    details: format!("Manual review required: {}", description),
                    executed_at: now,
                }
            }
        }
    }
}
