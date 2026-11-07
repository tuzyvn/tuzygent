mod engine;
mod environment;
mod executor;
mod output;

pub use engine::CallbackMockEngine;
pub use environment::TestEnv;
pub use executor::{
    setup_test_executor, setup_test_executor_with_timeout, try_setup_test_executor,
};

pub(super) fn mock_ok(stdout: &str) -> agent_engine::EngineOutput {
    output::mock_ok(stdout)
}

pub(super) fn mock_remediation(id: &str, title: &str, cmd: &str) -> agent_engine::EngineOutput {
    output::mock_remediation(id, title, cmd)
}
