use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

fn default_engine_name() -> String {
    "antigravity".to_string()
}

fn default_timeout() -> u64 {
    300
}

fn default_sandbox() -> String {
    "docker".to_string()
}

fn default_sandbox_image() -> String {
    "tuzygent-engine:latest".into()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    #[serde(default = "default_engine_name")]
    pub engine: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_sandbox")]
    pub sandbox_driver: String,
    #[serde(default = "default_sandbox_image")]
    pub sandbox_image: String,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            engine: default_engine_name(),
            timeout_seconds: default_timeout(),
            sandbox_driver: default_sandbox(),
            sandbox_image: default_sandbox_image(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMount {
    pub source: String,
    pub target: String,
    #[serde(default = "default_true")]
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default)]
    pub allowed_env_vars: Vec<String>,
    #[serde(default)]
    pub mount_credentials: Vec<CredentialMount>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_env_vars: vec![
                "CODEX_API_KEY".to_string(),
                "CODEX_BIN".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
                "GEMINI_API_KEY".to_string(),
                "OPENAI_API_KEY".to_string(),
                "ANTIGRAVITY_API_KEY".to_string(),
                "AGY_BIN".to_string(),
            ],
            mount_credentials: vec![
                CredentialMount {
                    source: "~/.codex/auth.json".into(),
                    target: "/root/.codex/auth.json".into(),
                    readonly: true,
                },
                CredentialMount {
                    source: "~/.gemini".to_string(),
                    target: "/root/.gemini".to_string(),
                    readonly: true,
                },
                CredentialMount {
                    source: "~/.claude".to_string(),
                    target: "/root/.claude".to_string(),
                    readonly: true,
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleEngineConfig {
    pub kind: String, // "antigravity", "claude_code", "codex", "generic", "openai", "mock"
    pub binary_path: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub model: Option<String>,
    pub interactive_fallback: Option<String>,
    pub output_parser: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnginesConfig {
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub engines: HashMap<String, SingleEngineConfig>,
}

impl Default for EnginesConfig {
    fn default() -> Self {
        let mut engines = HashMap::new();

        engines.insert(
            "antigravity".to_string(),
            SingleEngineConfig {
                kind: "antigravity".to_string(),
                binary_path: None,
                args: vec![
                    "--interactive=false".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--message".to_string(),
                    "{prompt}".to_string(),
                ],
                model: None,
                interactive_fallback: Some("tmux".to_string()),
                output_parser: Some("json_stream".to_string()),
                env: HashMap::new(),
            },
        );

        engines.insert(
            "claude".to_string(),
            SingleEngineConfig {
                kind: "claude_code".to_string(),
                binary_path: None,
                args: vec![
                    "-p".to_string(),
                    "{prompt}".to_string(),
                    "--output-format".to_string(),
                    "json".to_string(),
                ],
                model: None,
                interactive_fallback: Some("tmux".to_string()),
                output_parser: Some("json".to_string()),
                env: HashMap::new(),
            },
        );

        engines.insert(
            "codex".into(),
            SingleEngineConfig {
                kind: "codex".into(),
                binary_path: None,
                args: vec![],
                model: None,
                interactive_fallback: None,
                output_parser: Some("codex_jsonl".into()),
                env: HashMap::new(),
            },
        );

        Self {
            defaults: DefaultsConfig::default(),
            security: SecurityConfig::default(),
            engines,
        }
    }
}

impl EnginesConfig {
    /// Load hierarchical configuration:
    /// 1. Sane built-in defaults
    /// 2. Global OS config: ~/.tuzygent/engines.toml
    /// 3. Workspace local config: <workspace>/tuzygent.toml or .tuzygent.toml
    pub fn load_hierarchical(workspace_path: Option<&Path>) -> Self {
        let mut config = Self::default();

        // 1. Check Global OS config
        if let Some(home) = dirs_home() {
            let global_path = home.join(".tuzygent").join("engines.toml");
            if global_path.exists() {
                info!("Loading global OS engine config from {:?}", global_path);
                if let Ok(content) = std::fs::read_to_string(&global_path) {
                    if config.merge_toml(&content).is_ok() {
                    } else {
                        warn!("Failed to parse global engines.toml at {:?}", global_path);
                    }
                }
            }
        }

        // 2. Check Workspace local config
        if let Some(ws) = workspace_path {
            let local_candidates = [
                ws.join("tuzygent.toml"),
                ws.join(".tuzygent.toml"),
                ws.join(".tuzygent").join("engines.toml"),
            ];

            for path in &local_candidates {
                if path.exists() {
                    info!("Loading local workspace engine config from {:?}", path);
                    if let Ok(content) = std::fs::read_to_string(path) {
                        if config.merge_toml(&content).is_ok() {
                            break;
                        } else {
                            warn!("Failed to parse local config at {:?}", path);
                        }
                    }
                }
            }
        }

        config
    }

    /// Overlay only explicitly present keys. Empty arrays intentionally clear policy.
    fn merge_toml(&mut self, content: &str) -> Result<(), String> {
        fn overlay(base: &mut toml::Value, patch: toml::Value) {
            match (base, patch) {
                (toml::Value::Table(base), toml::Value::Table(patch)) => {
                    for (key, value) in patch {
                        if let Some(current) = base.get_mut(&key) {
                            overlay(current, value);
                        } else {
                            base.insert(key, value);
                        }
                    }
                }
                (base, value) => *base = value,
            }
        }
        let patch: toml::Value = toml::from_str(content).map_err(|error| error.to_string())?;
        let mut merged = toml::Value::try_from(&*self).map_err(|error| error.to_string())?;
        overlay(&mut merged, patch);
        let parsed = merged
            .try_into()
            .map_err(|error: toml::de::Error| error.to_string())?;
        *self = parsed;
        Ok(())
    }

    /// Merge settings from another config (overriding self)
    pub fn merge_from(&mut self, other: EnginesConfig) {
        if !other.defaults.engine.is_empty() {
            self.defaults.engine = other.defaults.engine;
        }
        if other.defaults.timeout_seconds > 0 {
            self.defaults.timeout_seconds = other.defaults.timeout_seconds;
        }
        if !other.defaults.sandbox_driver.is_empty() {
            self.defaults.sandbox_driver = other.defaults.sandbox_driver;
        }

        if !other.defaults.sandbox_image.is_empty() {
            self.defaults.sandbox_image = other.defaults.sandbox_image;
        }

        if !other.security.allowed_env_vars.is_empty() {
            self.security.allowed_env_vars = other.security.allowed_env_vars;
        }
        if !other.security.mount_credentials.is_empty() {
            self.security.mount_credentials = other.security.mount_credentials;
        }

        for (k, v) in other.engines {
            self.engines.insert(k, v);
        }
    }

    /// Convert self to TOML formatted string
    pub fn to_toml_string(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engines_config_serialization() {
        let config = EnginesConfig::default();
        let toml_str = config.to_toml_string().expect("Must serialize to toml");
        assert!(toml_str.contains("antigravity"));
        assert!(toml_str.contains("claude"));

        let parsed: EnginesConfig = toml::from_str(&toml_str).expect("Must parse from toml");
        assert_eq!(parsed.defaults.engine, "antigravity");
        assert_eq!(parsed.engines.len(), 3);
    }

    #[test]
    fn test_merge_config() {
        let mut base = EnginesConfig::default();
        let custom_toml = r#"
            [defaults]
            engine = "ollama"
            timeout_seconds = 120

            [engines.ollama]
            kind = "generic"
            binary_path = "ollama"
            args = ["run", "qwen2.5-coder:7b", "{prompt}"]
        "#;
        let custom: EnginesConfig = toml::from_str(custom_toml).unwrap();
        base.merge_from(custom);

        assert_eq!(base.defaults.engine, "ollama");
        assert_eq!(base.defaults.timeout_seconds, 120);
        assert!(base.engines.contains_key("ollama"));
    }
    #[test]
    fn partial_workspace_config_preserves_global_policy_and_can_clear_credentials() {
        let mut config = EnginesConfig::default();
        config.defaults.timeout_seconds = 17;
        config.defaults.sandbox_image = "private/engines:v1".into();
        config
            .merge_toml(
                r#"
            [engines.probe]
            kind = "generic"
            binary_path = "echo"
        "#,
            )
            .unwrap();
        assert_eq!(config.defaults.timeout_seconds, 17);
        assert_eq!(config.defaults.sandbox_image, "private/engines:v1");
        config
            .merge_toml(
                r#"
            [security]
            allowed_env_vars = []
            mount_credentials = []
        "#,
            )
            .unwrap();
        assert!(config.security.allowed_env_vars.is_empty());
        assert!(config.security.mount_credentials.is_empty());
    }
}
