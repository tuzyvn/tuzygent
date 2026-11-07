pub mod agent;
pub mod auth;
pub mod benchmark;
pub mod demo;
pub mod engines;
pub mod inspect;
pub mod memory;
pub mod persona;
pub mod replay;
pub mod resilience;
pub mod rig;
pub mod run;
pub mod workflow;

use crate::cli::{Cli, Commands};
use crate::fault::CliExecutionError;
use crate::resilience::ResilientCliRunner;

pub use benchmark::{CommandsBenchmarkHarness, CommandsBenchmarkStats};
pub use replay::{
    execute_replay_or_fork, inspect_checkpoints, CheckpointListPayload, ForkArgs, ReplayArgs,
    ReplayOutcomePayload,
};
pub use resilience::{ResilientCommandDispatcher, ShardedResilientCommandDispatcher};

pub async fn run_command(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Inspect(args) => inspect::handle(args).await?,
        Commands::Memory(args) => memory::handle(args).await?,
        Commands::Replay(args) => replay::handle(args).await?,
        Commands::Fork(args) => replay::handle_fork(args).await?,
        Commands::Update { check } => crate::update::run(check)?,
        Commands::Create { id, name, autonomy } => agent::create(id, name, autonomy).await?,
        Commands::List => agent::list().await?,
        Commands::Run {
            agent_id,
            goal,
            workspace,
            engine,
            sandbox,
            persona_path,
            timeout,
            auto_approve,
        } => {
            run::handle(run::RunArgs {
                agent_id,
                goal,
                workspace,
                engine,
                sandbox,
                persona_path,
                timeout,
                auto_approve,
            })
            .await?
        }
        Commands::Engines { workspace, test } => engines::handle(workspace, test).await?,
        Commands::RunWorkflow {
            workflow_id,
            catalog_path,
            workspace,
        } => workflow::handle(workflow_id, catalog_path, workspace).await?,
        Commands::DemoMultiAgent { engine, workspace } => {
            demo::multi_agent(engine, workspace).await?
        }
        Commands::ComedyShow { engine } => demo::comedy_show(engine).await?,
        Commands::PersonaInspect { path } => persona::inspect(path).await?,
        Commands::InitSamplePersonas { output_dir } => persona::init_samples(output_dir).await?,
        Commands::Rig { action } => rig::handle(action).await?,
        Commands::Auth { action } => auth::handle(action).await?,
    }

    Ok(())
}

/// Dispatches a command resiliently with automatic retry, fault injection handling,
/// panic isolation, and audit ledger tracking.
pub async fn run_command_resilient(
    cli: Cli,
    runner: &mut ResilientCliRunner,
) -> Result<(), CliExecutionError> {
    runner.run_command_resilient(cli).await
}

/// Dispatches a command using default resilience policy
pub async fn execute_command_resilient(cli: Cli) -> Result<(), CliExecutionError> {
    let mut runner = ResilientCliRunner::default();
    runner.run_command_resilient(cli).await
}
