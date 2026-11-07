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
    fn single_engine_config_serialization_and_defaults() {
        let config = SingleEngineConfig {
            kind: "generic".into(),
            binary_path: Some("/usr/bin/runner".into()),
            args: vec!["--model".into(), "{prompt}".into()],
            model: Some("model-v1".into()),
            interactive_fallback: Some("tmux".into()),
            output_parser: Some("json".into()),
            env: [("MODE".into(), "safe".into())].into(),
        };

        let serialized = serde_json::to_value(&config).expect("Must serialize config");
        assert_eq!(
            serialized,
            serde_json::json!({
                "kind": "generic",
                "binary_path": "/usr/bin/runner",
                "args": ["--model", "{prompt}"],
                "model": "model-v1",
                "interactive_fallback": "tmux",
                "output_parser": "json",
                "env": {"MODE": "safe"}
            })
        );

        let parsed: SingleEngineConfig =
            serde_json::from_value(serialized.clone()).expect("Must deserialize config");
        let round_tripped = serde_json::to_value(parsed).expect("Must serialize parsed config");
        assert_eq!(round_tripped, serialized);

        let minimal: SingleEngineConfig = toml::from_str("kind = \"mock\"")
            .expect("Must deserialize minimal config with defaults");
        assert_eq!(minimal.kind, "mock");
        assert_eq!(minimal.binary_path, None);
        assert!(minimal.args.is_empty());
        assert_eq!(minimal.model, None);
        assert_eq!(minimal.interactive_fallback, None);
        assert_eq!(minimal.output_parser, None);
        assert!(minimal.env.is_empty());
    }

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
        config.merge_toml(
            r#"
            [engines.probe]
            kind = "generic"
            binary_path = "echo"
        "#,
        )?;
        assert_eq!(config.defaults.timeout_seconds, 17);
        assert_eq!(config.defaults.sandbox_image, "private/engines:v1");
        config.merge_toml(
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
