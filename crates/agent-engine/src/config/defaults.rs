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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CredentialMount;

    #[test]
    fn default_engine_name_is_antigravity() {
        assert_eq!(default_engine_name(), "antigravity");
    }

    #[test]
    fn default_timeout_is_300_seconds() {
        assert_eq!(default_timeout(), 300);
    }

    #[test]
    fn default_sandbox_is_docker() {
        assert_eq!(default_sandbox(), "docker");
    }

    #[test]
    fn default_sandbox_image_is_latest_engine_image() {
        assert_eq!(default_sandbox_image(), "tuzygent-engine:latest");
    }

    #[test]
    fn default_true_is_true_and_is_used_for_missing_readonly() {
        assert!(default_true());

        let mount: CredentialMount = toml::from_str(
            r#"
                source = "/host/credentials"
                target = "/container/credentials"
            "#,
        )
        .expect("credential mount without readonly should use its default");
        assert!(mount.readonly);
    }

    #[test]
    fn defaults_config_deserialization_fills_missing_values() {
        let config: DefaultsConfig = toml::from_str("").expect("empty defaults config is valid");

        assert_eq!(config.engine, "antigravity");
        assert_eq!(config.timeout_seconds, 300);
        assert_eq!(config.sandbox_driver, "docker");
        assert_eq!(config.sandbox_image, "tuzygent-engine:latest");
    }
}
