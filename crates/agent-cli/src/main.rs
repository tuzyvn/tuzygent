use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = agent_cli::Cli::parse();
    agent_cli::run_command(cli).await
}
