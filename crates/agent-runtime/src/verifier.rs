use agent_domain::{VerificationResult, VerificationSpec};
use std::path::Path;
use tracing::{debug, info, warn};

pub struct TaskVerifier<'a> {
    workspace_dir: &'a Path,
}

impl<'a> TaskVerifier<'a> {
    pub fn new(workspace_dir: &'a Path) -> Self {
        Self { workspace_dir }
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
            } else if lower.starts_with("cargo test") || lower.starts_with("npm test") || lower.starts_with("pytest") {
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
            } else if lower.starts_with("json:") || (lower.contains("valid json") && lower.contains('.')) {
                let path = line.split_once(':').map(|(_, p)| p.trim()).unwrap_or(line);
                let cleaned_path = path.split_whitespace().next().unwrap_or(path);
                specs.push(VerificationSpec::JsonValid {
                    path: cleaned_path.to_string(),
                });
            } else if lower.starts_with("contains:") {
                let parts: Vec<&str> = line.split_once(':').map(|(_, rest)| rest).unwrap_or("").split('|').collect();
                if parts.len() >= 2 {
                    specs.push(VerificationSpec::ContentContains {
                        path: parts[0].trim().to_string(),
                        pattern: parts[1].trim().to_string(),
                    });
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
                                details: format!("File {:?} contains required pattern '{}'", path, pattern),
                                executed_at: now,
                            }
                        } else {
                            VerificationResult {
                                passed: false,
                                spec: spec.clone(),
                                details: format!("File {:?} does not contain required pattern '{}'", path, pattern),
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
            VerificationSpec::Command { command, expected_exit_code } => {
                info!("Executing verification command: {}", command);
                let mut cmd = tokio::process::Command::new("sh");
                cmd.arg("-c").arg(command);
                cmd.current_dir(self.workspace_dir);

                match cmd.output().await {
                    Ok(output) => {
                        let actual_code = output.status.code().unwrap_or(-1);
                        let passed = actual_code == *expected_exit_code;
                        let stdout_tail: String = String::from_utf8_lossy(&output.stdout).chars().rev().take(500).collect::<String>().chars().rev().collect();
                        let stderr_tail: String = String::from_utf8_lossy(&output.stderr).chars().rev().take(500).collect::<String>().chars().rev().collect();
                        VerificationResult {
                            passed,
                            spec: spec.clone(),
                            details: format!(
                                "Exit code: {} (expected {}). stdout: {} | stderr: {}",
                                actual_code, expected_exit_code, stdout_tail.trim(), stderr_tail.trim()
                            ),
                            executed_at: now,
                        }
                    }
                    Err(e) => VerificationResult {
                        passed: false,
                        spec: spec.clone(),
                        details: format!("Failed to spawn verification command '{}': {}", command, e),
                        executed_at: now,
                    },
                }
            }
            VerificationSpec::Custom { description } => {
                debug!("Custom verification note: {}", description);
                VerificationResult {
                    passed: true,
                    spec: spec.clone(),
                    details: format!("Custom verification acknowledged: {}", description),
                    executed_at: now,
                }
            }
        }
    }

    /// Verifies all specifications for a subtask
    pub async fn verify_all(&self, specs: &[VerificationSpec]) -> (bool, Vec<VerificationResult>) {
        let mut results = Vec::new();
        let mut all_passed = true;
        for spec in specs {
            let res = self.verify_spec(spec).await;
            if !res.passed {
                warn!("Verification check failed: {} -> {}", spec_description(spec), res.details);
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
        VerificationSpec::ContentContains { path, pattern } => format!("ContentContains in '{}' pattern '{}'", path, pattern),
        VerificationSpec::Custom { description } => format!("Custom '{}'", description),
    }
}
