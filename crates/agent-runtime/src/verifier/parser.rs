use agent_domain::VerificationSpec;

use super::TaskVerifier;

impl TaskVerifier<'_> {
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
}
