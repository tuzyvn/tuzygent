//! A minimal SDK delegation, using a preinstalled and authenticated Codex CLI.
//! Pass an explicit binary path to avoid invoking package-manager update shims.
use agent_engine::{CodexEngine, ComputeEngine, EngineInput};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let binary = arguments.next();
    let workspace = arguments.next().unwrap_or_else(|| ".".into());
    let session_id = arguments.next();
    let engine = CodexEngine::new(binary, None);
    let mut input = EngineInput::new(
        "Return exactly TUZYGENT_CODEX_OK. Do not inspect files, call tools, or change anything.",
        workspace,
    );
    input.timeout_seconds = 60;
    let output = if let Some(session_id) = session_id {
        engine.resume(&input, &session_id).await?
    } else {
        engine.execute(&input).await?
    };
    println!(
        "engine={} exit={} session={}",
        output.engine_used,
        output.exit_code,
        output.session_id.as_deref().unwrap_or("none")
    );
    println!("{}", output.stdout_content);
    if output.exit_code != 0 || output.stdout_content.trim() != "TUZYGENT_CODEX_OK" {
        std::process::exit(1);
    }
    Ok(())
}
