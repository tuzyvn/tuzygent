use serde::{Deserialize, Serialize};

pub fn default_engine_name() -> String {
    "antigravity".to_string()
}

pub fn default_timeout() -> u64 {
    300
}

pub fn default_sandbox() -> String {
    "docker".to_string()
}

pub fn default_sandbox_image() -> String {
    "tuzygent-engine:latest".into()
}

pub fn default_true() -> bool {
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
