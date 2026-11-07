use super::fixtures::*;
use agent_domain::GuardrailPolicy;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_env_lifecycle_and_try_new() -> anyhow::Result<()> {
        let env = TestEnv::try_new("test_env_fixture_success")?;
        assert!(env.path().exists() && env.path().join("verified_fixture").is_file());
        assert_eq!(
            std::fs::read_to_string(env.path().join("verified_fixture"))?,
            "test evidence"
        );
        Ok(())
    }

    #[test]
    fn test_test_env_panic_free_and_helpers() -> anyhow::Result<()> {
        let mut env = TestEnv::new("test_env_helpers");
        let sub = env.create_sub_dir("nested/deep")?;
        assert!(sub.is_dir());
        let written = env.write_file("nested/deep/agent.txt", "agent operational data")?;
        assert!(written.is_file() && env.file_exists("nested/deep/agent.txt"));
        assert_eq!(
            env.read_file("nested/deep/agent.txt")?,
            "agent operational data"
        );
        env.remove_file("nested/deep/agent.txt")?;
        assert!(!env.file_exists("nested/deep/agent.txt"));
        env.clean()?;
        assert!(!env.path().exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_callback_mock_engine_and_executors() -> anyhow::Result<()> {
        let mock_engine = Arc::new(CallbackMockEngine::with_constant_output("hello response"));
        let input = EngineInput::new("echo hello", "/tmp");
        let output = mock_engine.execute(&input).await?;
        assert_eq!(output.stdout_content, "hello response");
        assert_eq!(mock_engine.call_count(), 1);
        mock_engine.reset();
        assert_eq!(mock_engine.call_count(), 0);

        let timeout_engine = Arc::new(CallbackMockEngine::with_timeout());
        assert!(matches!(
            timeout_engine.execute(&input).await,
            Err(EngineError::Timeout)
        ));

        let fail_engine = Arc::new(CallbackMockEngine::with_error("simulated fatal failure"));
        assert!(matches!(
            fail_engine.execute(&input).await,
            Err(EngineError::ProcessFailed(_))
        ));

        let exit_engine = Arc::new(CallbackMockEngine::with_exit_code(42, "custom exit"));
        let exit_out = exit_engine.execute(&input).await?;
        assert_eq!(exit_out.exit_code, 42);

        let seq_engine = Arc::new(CallbackMockEngine::with_sequence(vec![Ok(EngineOutput {
            stdout_content: "first".into(),
            exit_code: 0,
            execution_duration_ms: 5,
            engine_used: EngineKind::Mock,
            session_id: None,
        })]));
        assert_eq!(seq_engine.execute(&input).await?.stdout_content, "first");
        assert!(seq_engine
            .execute(&input)
            .await?
            .stdout_content
            .contains("sequence exhausted"));

        let env = TestEnv::new("test_exec_env");
        let executor_zero =
            setup_test_executor_with_timeout(mock_engine.clone(), None, env.path(), 0);
        assert_eq!(executor_zero.timeout_seconds, 30);

        let try_exec = try_setup_test_executor(
            mock_engine,
            Some(GuardrailPolicy::default()),
            env.path(),
            Some(0),
        )?;
        assert_eq!(try_exec.timeout_seconds, 30);
        Ok(())
    }
}
