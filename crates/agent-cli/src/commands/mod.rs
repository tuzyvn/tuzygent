pub mod agent;
pub mod auth;
pub mod demo;
pub mod engines;
pub mod inspect;
pub mod memory;
pub mod persona;
pub mod rig;
pub mod run;
pub mod workflow;

use crate::cli::{Cli, Commands};

pub async fn run_command(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Inspect(args) => inspect::handle(args).await?,
        Commands::Memory(args) => memory::handle(args).await?,
        Commands::Update { check } => crate::update::run(check)?,
        Commands::Create { id, name, autonomy } => {
            agent::create(id, name, autonomy).await?
        }
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
        Commands::InitSamplePersonas { output_dir } => {
            persona::init_samples(output_dir).await?
        }
        Commands::Rig { action } => rig::handle(action).await?,
        Commands::Auth { action } => auth::handle(action).await?,
    }

    Ok(())
}
