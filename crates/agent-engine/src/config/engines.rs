use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::defaults::DefaultsConfig;
use super::security::SecurityConfig;

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
