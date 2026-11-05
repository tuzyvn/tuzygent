use agent_core::prelude::*;
use clap::Parser;
use std::sync::Arc;

#[derive(Parser, Debug, Clone)]
#[command(name = "agent-server")]
#[command(about = "AgentCore Standalone REST & WebSocket Daemon Server")]
pub struct ServerCli {
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    pub host: String,

    #[arg(short, long, default_value_t = 8080)]
    pub port: u16,

    #[arg(short, long, default_value = ".")]
    pub workspace: String,

    #[arg(long, env = "TUZYGENT_API_TOKEN", hide_env_values = true)]
    pub api_token: String,

    #[arg(long, env = "TUZYGENT_ALLOWED_ORIGINS", value_delimiter = ',')]
    pub allowed_origins: Vec<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub supervisor: Arc<AgentSupervisor>,
    pub bus: InterAgentBus,
    pub workspace: String,
    pub vault: Arc<agent_engine::AccountVault>,
    pub task_service: Arc<agent_runtime::TaskService>,
}
