use clap::Parser;

mod cli;
mod commands;
mod update;

pub use cli::{AuthAction, Cli, Commands, RigAction};
pub use commands::memory;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Memory(args) => commands::memory::handle(args).await?,
        Commands::Update { check } => update::run(check)?,
        Commands::Create { id, name, autonomy } => {
            commands::agent::create(id, name, autonomy).await?
        }
        Commands::List => commands::agent::list().await?,
        Commands::Run {
            agent_id,
            goal,
            workspace,
            engine,
            sandbox,
            persona_path,
            timeout,
        } => {
            commands::run::handle(commands::run::RunArgs {
                agent_id,
                goal,
                workspace,
                engine,
                sandbox,
                persona_path,
                timeout,
            })
            .await?
        }
        Commands::Engines { workspace, test } => commands::engines::handle(workspace, test).await?,
        Commands::RunWorkflow {
            workflow_id,
            catalog_path,
            workspace,
        } => commands::workflow::handle(workflow_id, catalog_path, workspace).await?,
        Commands::DemoMultiAgent { engine, workspace } => {
            commands::demo::multi_agent(engine, workspace).await?
        }
        Commands::ComedyShow { engine } => commands::demo::comedy_show(engine).await?,
        Commands::PersonaInspect { path } => commands::persona::inspect(path).await?,
        Commands::InitSamplePersonas { output_dir } => {
            commands::persona::init_samples(output_dir).await?
        }
        Commands::Rig { action } => commands::rig::handle(action).await?,
        Commands::Auth { action } => commands::auth::handle(action).await?,
    }

    Ok(())
}
