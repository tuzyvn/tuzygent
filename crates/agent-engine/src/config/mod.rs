pub mod defaults;
pub mod engines;
pub mod loader;
pub mod security;

pub use defaults::*;
pub use engines::*;
pub use loader::*;
pub use security::*;

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
    fn test_merge_config() -> Result<(), Box<dyn std::error::Error>> {
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
        let custom: EnginesConfig = toml::from_str(custom_toml)?;
        base.merge_from(custom);

        assert_eq!(base.defaults.engine, "ollama");
        assert_eq!(base.defaults.timeout_seconds, 120);
        assert!(base.engines.contains_key("ollama"));
        Ok(())
    }

    #[test]
    fn partial_workspace_config_preserves_global_policy_and_can_clear_credentials(
    ) -> Result<(), Box<dyn std::error::Error>> {
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
            )?;
        assert_eq!(config.defaults.timeout_seconds, 17);
        assert_eq!(config.defaults.sandbox_image, "private/engines:v1");
        config
            .merge_toml(
                r#"
            [security]
            allowed_env_vars = []
            mount_credentials = []
        "#,
            )?;
        assert!(config.security.allowed_env_vars.is_empty());
        assert!(config.security.mount_credentials.is_empty());
        Ok(())
    }

    #[test]
    fn test_merge_toml_invalid_syntax_returns_err() {
        let mut config = EnginesConfig::default();
        let invalid_toml = "invalid toml ::: [syntax]";
        let result = config.merge_toml(invalid_toml);
        assert!(result.is_err());
    }
}
