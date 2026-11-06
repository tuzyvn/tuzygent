use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = match agent_cli::Cli::try_parse() {
        Ok(c) => c,
        Err(err) => {
            err.exit();
        }
    };

    let mut runner = agent_cli::ResilientCliRunner::default();
    runner
        .run_command_resilient(cli)
        .await
        .map_err(|e| anyhow::anyhow!(e))
}
