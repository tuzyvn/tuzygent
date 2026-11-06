#![deny(ambiguous_glob_reexports)]
#![deny(clippy::clone_on_copy)]
#![deny(clippy::cloned_instead_of_copied)]
#![deny(suspicious_double_ref_op)]

pub mod benchmark;
pub mod cli;
pub mod commands;
pub mod fault;
pub mod resilience;
pub mod update;

pub use benchmark::{CliBenchmarkHarness, CliBenchmarkStats};
pub use cli::{
    exit_codes, AuthAction, Cli, CliExecutionBuffer, Commands, InspectArgs, InspectTarget,
    OutputFormat, RigAction, SafeExitCode,
};
pub use commands::{
    execute_command_resilient, run_command, run_command_resilient, CommandsBenchmarkHarness,
    CommandsBenchmarkStats, ResilientCommandDispatcher, ShardedResilientCommandDispatcher,
};
pub use fault::{
    is_retryable_cli_error, simulate_cli_fault, CliExecutionError, CliFault, CliFaultPolicy,
};
pub use resilience::{ResilientCliRunner, ShardedResilientCliRunner};
pub use update::{
    run as update_run, run_resilient_command, ResilientUpdater, ShardedResilientUpdater,
    TemporaryDirectory, UpdateBenchmarkHarness, UpdateBenchmarkStats, UpdateStatus,
};
