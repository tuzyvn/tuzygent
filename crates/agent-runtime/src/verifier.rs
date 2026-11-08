use agent_domain::{VerificationResult, VerificationSpec};
use std::path::Path;
use tracing::{debug, info, warn};

pub struct TaskVerifier<'a> {
    workspace_dir: &'a Path,
    sandbox: crate::sandbox::SandboxConfig,
    guardrail: agent_domain::GuardrailPolicy,
}

impl<'a> TaskVerifier<'a> {
    pub fn new(workspace_dir: &'a Path) -> Self {
        Self {
            workspace_dir,
            sandbox: Default::default(),
            guardrail: agent_domain::GuardrailPolicy::default_enterprise(),
        }
    }

    pub fn with_policy(
        workspace_dir: &'a Path,
        sandbox: crate::sandbox::SandboxConfig,
        guardrail: agent_domain::GuardrailPolicy,
    ) -> Self {
        Self {
            workspace_dir,
            sandbox,
            guardrail,
        }
    }

    /// Automatically parse string description of expected verification into concrete specifications
    pub fn parse_specs(&self, text: &str) -> Vec<VerificationSpec> {
        let mut specs = Vec::new();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return specs;
        }

        for line in trimmed.lines() {
            let line = line.trim().trim_start_matches(['-', '*', '•']).trim();
            if line.is_empty() {
                continue;
            }

            let lower = line.to_lowercase();
            if lower.starts_with("command:") || lower.starts_with("cmd:") {
                let cmd = line.split_once(':').map(|(_, c)| c.trim()).unwrap_or(line);
                specs.push(VerificationSpec::Command {
                    command: cmd.to_string(),
                    expected_exit_code: 0,
                });
            } else if lower.starts_with("cargo test")
                || lower.starts_with("npm test")
                || lower.starts_with("pytest")
            {
                specs.push(VerificationSpec::Command {
                    command: line.to_string(),
                    expected_exit_code: 0,
                });
            } else if lower.starts_with("file:") || lower.starts_with("file_exists:") {
                let path = line.split_once(':').map(|(_, p)| p.trim()).unwrap_or(line);
                specs.push(VerificationSpec::FileExists {
                    path: path.to_string(),
                    min_bytes: Some(1),
                });
            } else if lower.starts_with("json:")
                || (lower.contains("valid json") && lower.contains('.'))
            {
                let path = line.split_once(':').map(|(_, p)| p.trim()).unwrap_or(line);
                let cleaned_path = path.split_whitespace().next().unwrap_or(path);
                specs.push(VerificationSpec::JsonValid {
                    path: cleaned_path.to_string(),
                });
            } else if lower.starts_with("contains:") {
                let rest = line.split_once(':').map(|(_, r)| r.trim()).unwrap_or("");
                if let Some((path, pattern)) = rest.split_once('|') {
                    let path = path.trim();
                    let pattern = pattern.trim();
                    if !path.is_empty() && !pattern.is_empty() {
                        specs.push(VerificationSpec::ContentContains {
                            path: path.to_string(),
                            pattern: pattern.to_string(),
                        });
                    }
                }
            } else {
                specs.push(VerificationSpec::Custom {
                    description: line.to_string(),
                });
            }
        }

        if specs.is_empty() {
            specs.push(VerificationSpec::Custom {
                description: trimmed.to_string(),
            });
        }
        specs
    }

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

    /// Verifies all specifications for a subtask
    pub async fn verify_all(&self, specs: &[VerificationSpec]) -> (bool, Vec<VerificationResult>) {
        let mut results = Vec::new();
        let mut all_passed = !specs.is_empty();
        for spec in specs {
            let res = self.verify_spec(spec).await;
            if !res.passed {
                warn!(
                    "Verification check failed: {} -> {}",
                    spec_description(spec),
                    res.details
                );
                all_passed = false;
            }
            results.push(res);
        }
        (all_passed, results)
    }
}

fn spec_description(spec: &VerificationSpec) -> String {
    match spec {
        VerificationSpec::Command { command, .. } => format!("Command '{}'", command),
        VerificationSpec::FileExists { path, .. } => format!("FileExists '{}'", path),
        VerificationSpec::JsonValid { path } => format!("JsonValid '{}'", path),
        VerificationSpec::ContentContains { path, pattern } => {
            format!("ContentContains in '{}' pattern '{}'", path, pattern)
        }
        VerificationSpec::Custom { description } => format!("Custom '{}'", description),
    }
}

fn take_tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count > n {
        s.chars().skip(count - n).collect()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_specs_recognizes_supported_verification_descriptions() {
        let temp_dir = std::env::temp_dir();
        let verifier = TaskVerifier::new(&temp_dir);
        let specs = verifier.parse_specs(
            "- command: cargo check\n* file_exists: report.txt\n• json: data.json\ncontains: report.txt | finished\npytest tests/\nReview deployment manually",
        );

        assert_eq!(
            specs,
            vec![
                VerificationSpec::Command {
                    command: "cargo check".into(),
                    expected_exit_code: 0,
                },
                VerificationSpec::FileExists {
                    path: "report.txt".into(),
                    min_bytes: Some(1),
                },
                VerificationSpec::JsonValid {
                    path: "data.json".into(),
                },
                VerificationSpec::ContentContains {
                    path: "report.txt".into(),
                    pattern: "finished".into(),
                },
                VerificationSpec::Command {
                    command: "pytest tests/".into(),
                    expected_exit_code: 0,
                },
                VerificationSpec::Custom {
                    description: "Review deployment manually".into(),
                },
            ]
        );
        assert!(verifier.parse_specs(" \n ").is_empty());
    }

    #[tokio::test]
    async fn test_verify_all_aggregates_results_and_rejects_empty_specs() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
        let verifier = TaskVerifier::new(workspace);
        let specs = vec![
            VerificationSpec::FileExists {
                path: "Cargo.toml".into(),
                min_bytes: Some(1),
            },
            VerificationSpec::Custom {
                description: "manual review required".into(),
            },
        ];

        let (passed, results) = verifier.verify_all(&specs).await;
        assert!(!passed);
        assert_eq!(results.len(), 2);
        assert!(results[0].passed);
        assert!(!results[1].passed);

        let (empty_passed, empty_results) = verifier.verify_all(&[]).await;
        assert!(!empty_passed);
        assert!(empty_results.is_empty());
    }

    #[tokio::test]
    async fn test_verify_command_spec_failure_handled_gracefully() {
        let temp_dir = std::env::temp_dir();
        let verifier = TaskVerifier::new(&temp_dir);
        let spec = VerificationSpec::Command {
            command: "exit 42".to_string(),
            expected_exit_code: 0,
        };
        let res = verifier.verify_spec(&spec).await;
        assert!(!res.passed);
        assert!(res.details.contains("42"));
    }

    #[tokio::test]
    async fn test_verify_command_nonexistent_handled_gracefully() {
        let temp_dir = std::env::temp_dir();
        let verifier = TaskVerifier::new(&temp_dir);
        let spec = VerificationSpec::Command {
            command: "non_existent_binary_xyz_12345".to_string(),
            expected_exit_code: 0,
        };
        let res = verifier.verify_spec(&spec).await;
        assert!(!res.passed);
    }
}
