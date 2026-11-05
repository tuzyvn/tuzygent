use std::path::PathBuf;
use clap::{Parser, Subcommand};

use crate::commands::memory::MemoryArgs;

#[derive(Parser)]
#[command(name = "agent-ctl")]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), "\nCommit: ", env!("AGENTCTL_COMMIT"), "\nRelease date: ", env!("AGENTCTL_RELEASE_DATE")))]
#[command(about = "Enterprise Autonomous Multi-Agent Platform CLI", long_about = None)]
pub struct Cli {
    /// Print version, source commit and release date
    #[arg(short = 'v', action = clap::ArgAction::Version)]
    pub short_version: Option<bool>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Inspect task, memory, or workflow DAG details
    Inspect(InspectArgs),
    /// Manage project-scoped persona memory
    Memory(MemoryArgs),
    /// Build and install the latest AgentCore main from GitHub (requires Git and Cargo)
    Update {
        /// Only check the latest commit; do not build or replace the executable
        #[arg(long)]
        check: bool,
    },
    /// Create a new autonomous digital employee
    Create {
        #[arg(short, long)]
        id: String,
        #[arg(short, long)]
        name: String,
        #[arg(short, long, default_value = "full")]
        autonomy: String,
    },
    /// List all registered agents and their statuses
    List,
    /// Execute an autonomous mission using an agent
    Run {
        #[arg(short, long)]
        agent_id: String,
        #[arg(short, long)]
        goal: String,
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(short, long, default_value = "")]
        engine: String,
        #[arg(long)]
        sandbox: Option<String>,
        #[arg(short, long)]
        persona_path: Option<String>,
        #[arg(short, long)]
        timeout: Option<u64>,
        /// Automatically approve custom/unspecified verification if execution succeeds with exit code 0
        #[arg(long, default_value_t = true)]
        auto_approve: bool,
    },
    /// List, inspect or test configured AI CLI engines
    Engines {
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(long)]
        test: Option<String>,
    },
    /// Execute an entire multi-agent workflow DAG definition from catalog.json
    RunWorkflow {
        #[arg(long)]
        workflow_id: String,
        #[arg(long, default_value = "workflows/catalog.json")]
        catalog_path: String,
        #[arg(long, default_value = ".")]
        workspace: String,
    },
    /// Execute real multi-agent collaboration and task delegation via Event Bus
    #[command(alias = "multi-agent", alias = "collab")]
    DemoMultiAgent {
        #[arg(short, long, default_value = "")]
        engine: String,
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
    /// Comedy Show: Two autonomous agents performing a Stand-up comedy show about IT life
    ComedyShow {
        #[arg(short, long, default_value = "antigravity")]
        engine: String,
    },
    /// Load and inspect persona profile from directory or file
    PersonaInspect {
        #[arg(short, long)]
        path: String,
    },
    /// Initialize sample persona profiles (DevOps, Coder, ERP Auditor, HR Payroll)
    InitSamplePersonas {
        #[arg(short, long, default_value = "personas")]
        output_dir: String,
    },
    /// Manage multi-agent teams declaratively via OpenRig-style RigSpec
    Rig {
        #[command(subcommand)]
        action: RigAction,
    },
    /// Manage multiple CLI accounts, tokens and rotation policies
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Subcommand)]
pub enum AuthAction {
    /// List all configured accounts and their statuses
    List {
        #[arg(short, long)]
        engine: Option<String>,
    },
    /// Register or import an account into the vault
    Add {
        #[arg(short, long)]
        engine: String,
        #[arg(short, long)]
        id: String,
        #[arg(short, long)]
        name: String,
        #[arg(long)]
        api_key: Option<String>,
    },
    /// Remove an account from the vault
    Remove {
        #[arg(short, long)]
        engine: String,
        #[arg(short, long)]
        id: String,
    },
}

#[derive(Subcommand)]
pub enum RigAction {
    /// Boot all persistent seats defined in RigSpec (creates tmux sessions or processes)
    Up {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
    /// Shutdown and cleanup all seats of a Rig
    Down {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
    },
    /// Inspect health and status of all seats in a Rig
    Status {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
    },
    /// Deliver a high-level goal to the Kernel Operator for autonomous team dispatch
    Run {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
        #[arg(short, long)]
        goal: String,
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(short, long)]
        engine: Option<String>,
    },
    /// Create a full state snapshot of the Rig (seat states, tmux buffers, timestamps)
    Snapshot {
        #[arg(short, long, default_value = "rigs/software-dev.yaml")]
        spec: PathBuf,
        #[arg(short, long, default_value = ".")]
        workspace: String,
        #[arg(short, long)]
        out: Option<String>,
    },
    /// Restore a Rig's seats from a snapshot file
    Restore {
        #[arg(short, long)]
        snapshot: PathBuf,
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
}

#[derive(clap::Args, Debug, Clone)]
pub struct InspectArgs {
    /// Output format: table (default) or json
    #[arg(long, value_enum, default_value = "table", global = true)]
    pub format: OutputFormat,

    /// Target workspace root path
    #[arg(long, default_value = ".", global = true)]
    pub workspace: PathBuf,

    /// Inspection target: task | memory | dag
    #[command(subcommand)]
    pub target: InspectTarget,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
}

#[derive(Subcommand, Debug, Clone)]
pub enum InspectTarget {
    /// Inspect task state, subtasks execution details, and result
    #[command(alias = "task-id")]
    Task {
        /// Optional Task ID (positional)
        id: Option<String>,
        /// Optional Task ID (via flag --id)
        #[arg(long = "id")]
        flag_id: Option<String>,
        /// Filter by task status
        #[arg(long)]
        status: Option<String>,
        /// Max number of tasks to inspect/list
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Inspect agent persona memory records and retrieved lessons
    Memory {
        /// Optional Memory ID or semantic query
        id_or_query: Option<String>,
        /// Project scope
        #[arg(long, default_value = "default")]
        project: String,
        /// Persona name
        #[arg(long, default_value = "default")]
        persona: String,
    },
    /// Inspect workflow DAG definition, nodes, edges, or execution run report
    Dag {
        /// Optional workflow ID or run ID
        id: Option<String>,
        /// Path to workflow catalog JSON
        #[arg(long, default_value = "workflows/catalog.json")]
        catalog_path: String,
    },
}
