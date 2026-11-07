#![deny(ambiguous_glob_reexports)]
#![deny(clippy::clone_on_copy)]
#![deny(clippy::cloned_instead_of_copied)]
#![deny(suspicious_double_ref_op)]

pub mod accounts;
pub mod api;
pub mod backoff;
pub mod cli;
pub mod config;
pub mod process;
pub mod router;
pub mod tmux;
pub mod traits;

pub use accounts::chaos as accounts_chaos;
pub use accounts::{
    AccountBenchmarkHarness, AccountBenchmarkStats, AccountEngineKind, AccountError, AccountFault,
    AccountFaultPolicy, AccountProfile, AccountRotatedEngine, AccountStatus, AccountVault,
    ResilientAccountManager, SafeAccountProfile,
};
pub use api::chaos as api_chaos;
pub use api::{
    custom_api, is_retryable_api_error_message, is_retryable_engine_error, openai_compat,
    simulate_fault as simulate_api_fault, ApiBenchmarkHarness, ApiBenchmarkStats, ApiError,
    ApiFault, ApiFaultPolicy, BenchmarkMockApiEngine, CustomAgentApiEngine, OpenAiCompatibleEngine,
    ResilientApiEngine, ResilientApiManager, ResilientApiWrapper,
};
pub use backoff::BackoffPolicy;
pub use cli::chaos as cli_chaos;
pub use cli::{
    antigravity, claude_code, codex, generic, is_retryable_cli_error_message,
    is_retryable_engine_error as is_retryable_cli_engine_error, mock, simulate_cli_fault,
    AntigravityEngine, BenchmarkMockCliEngine, ClaudeCodeEngine, CliBenchmarkHarness,
    CliBenchmarkStats, CliError, CliFault, CliFaultPolicy, CodexEngine, GenericCliEngine,
    MockEngine, ResilientCliEngine, ResilientCliManager, ResilientCliWrapper,
};
pub use config::*;
pub use router::*;
pub use tmux::*;
pub use traits::*;
