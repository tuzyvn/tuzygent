use clap::{Parser, Subcommand};
use std::collections::VecDeque;
use std::path::PathBuf;

use crate::commands::memory::MemoryArgs;
use crate::commands::replay::{ForkArgs, ReplayArgs};

#[derive(Parser, Debug, Clone)]
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

impl Cli {
    /// Resilient parsing that catches Clap argument errors and returns structured CliExecutionError
    pub fn try_parse_resilient<I, T>(itr: I) -> Result<Self, crate::fault::CliExecutionError>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::try_parse_from(itr).map_err(|err| crate::fault::CliExecutionError::ParseError {
            code: err.exit_code(),
            message: err.to_string(),
        })
    }

    /// Parse arguments safely from environment without raw abort
    pub fn try_parse_safe() -> Result<Self, crate::fault::CliExecutionError> {
        Self::try_parse_resilient(std::env::args_os())
    }

    /// Evaluates whether an exit code is intermittent or retryable
    pub fn is_intermittent_exit_code(code: i32) -> bool {
        matches!(
            code,
            2 | exit_codes::TRANSIENT_INTERMITTENT | 111 | exit_codes::TIMEOUT | 137
        )
    }

    /// Normalizes and controls raw exit code safely to prevent crash loops
    pub fn sanitize_exit_code(raw_code: i32) -> i32 {
        if raw_code == exit_codes::SUCCESS {
            exit_codes::SUCCESS
        } else if Self::is_intermittent_exit_code(raw_code) {
            exit_codes::TRANSIENT_INTERMITTENT
        } else {
            raw_code
        }
    }

    /// Validates core CLI command arguments to prevent invalid execution or panics
    pub fn validate(&self) -> Result<(), crate::fault::CliExecutionError> {
        match &self.command {
            Commands::Run { agent_id, goal, .. } => {
                if agent_id.trim().is_empty() {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "agent_id must not be empty".to_string(),
                    });
                }
                if goal.trim().is_empty() {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "goal must not be empty".to_string(),
                    });
                }
                Ok(())
            }
            Commands::Create { id, name, .. } => {
                if id.trim().is_empty() || name.trim().is_empty() {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "id and name must not be empty".to_string(),
                    });
                }
                Ok(())
            }
            Commands::RunWorkflow { workflow_id, .. } => {
                if workflow_id.trim().is_empty() {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "workflow_id must not be empty".to_string(),
                    });
                }
                Ok(())
            }
            Commands::Replay(args) => {
                if args.resolved_task_id().is_none() {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "task_id must not be empty".to_string(),
                    });
                }
                if args.from_step == Some(0) {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "from_step must be >= 1".to_string(),
                    });
                }
                if args
                    .subtask_id
                    .as_deref()
                    .is_some_and(|s| s.trim().is_empty())
                {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "subtask_id must not be empty".to_string(),
                    });
                }
                Ok(())
            }
            Commands::Fork(args) => {
                if args.resolved_task_id().is_none() {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "task_id must not be empty".to_string(),
                    });
                }
                if args.from_step == Some(0) {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "from_step must be >= 1".to_string(),
                    });
                }
                if args
                    .subtask_id
                    .as_deref()
                    .is_some_and(|s| s.trim().is_empty())
                {
                    return Err(crate::fault::CliExecutionError::ParseError {
                        code: exit_codes::USAGE_ERROR,
                        message: "subtask_id must not be empty".to_string(),
                    });
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Intercepts and catches intermittent errors, mapping them into structured CliExecutionError
    pub fn catch_intermittent_error<T, E>(
        res: Result<T, E>,
    ) -> Result<T, crate::fault::CliExecutionError>
    where
        E: ToString,
    {
        res.map_err(|err| {
            let msg = err.to_string();
            if msg.contains("timed out") || msg.contains("timeout") {
                crate::fault::CliExecutionError::SocketTimeout
            } else if msg.contains("connection reset") {
                crate::fault::CliExecutionError::ConnectionReset
            } else if msg.contains("broken pipe") {
                crate::fault::CliExecutionError::BrokenPipe
            } else if msg.contains("non-zero code 2") {
                crate::fault::CliExecutionError::NonZeroExit {
                    code: 2,
                    message: msg,
                }
            } else if msg.contains("lock") {
                crate::fault::CliExecutionError::LockContention(msg)
            } else {
                crate::fault::CliExecutionError::Other(msg)
            }
        })
    }
}

/// Bounded buffer for queuing and buffering CLI operations with strictly controlled capacity and FIFO eviction
#[derive(Debug, Clone)]
pub struct CliExecutionBuffer<T> {
    buffer: VecDeque<T>,
    capacity: usize,
    dropped_count: usize,
}

impl<T> CliExecutionBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            buffer: VecDeque::with_capacity(cap),
            capacity: cap,
            dropped_count: 0,
        }
    }

    pub fn push(&mut self, item: T) -> Option<T> {
        let mut evicted = None;
        if self.buffer.len() >= self.capacity {
            evicted = self.buffer.pop_front();
            self.dropped_count += 1;
        }
        self.buffer.push_back(item);
        evicted
    }

    pub fn pop(&mut self) -> Option<T> {
        self.buffer.pop_front()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.buffer.len() >= self.capacity
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn dropped_count(&self) -> usize {
        self.dropped_count
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn drain_all(&mut self) -> Vec<T> {
        self.buffer.drain(..).collect()
    }
}

/// Standard safe CLI exit codes
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const GENERAL_ERROR: i32 = 1;
    pub const USAGE_ERROR: i32 = 2;
    pub const TRANSIENT_INTERMITTENT: i32 = 75; // EX_TEMPFAIL
    pub const TIMEOUT: i32 = 124;
    pub const TERMINATED_SIGNAL: i32 = 130;
}

/// Safe exit code abstraction classifying process termination reasons
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeExitCode {
    Success,
    GeneralFailure,
    UsageError,
    IntermittentFailure,
    Timeout,
    Custom(i32),
}

impl SafeExitCode {
    pub fn as_i32(&self) -> i32 {
        match self {
            Self::Success => exit_codes::SUCCESS,
            Self::GeneralFailure => exit_codes::GENERAL_ERROR,
            Self::UsageError => exit_codes::USAGE_ERROR,
            Self::IntermittentFailure => exit_codes::TRANSIENT_INTERMITTENT,
            Self::Timeout => exit_codes::TIMEOUT,
            Self::Custom(c) => *c,
        }
    }

    pub fn from_raw(code: i32) -> Self {
        match code {
            exit_codes::SUCCESS => Self::Success,
            exit_codes::GENERAL_ERROR => Self::GeneralFailure,
            exit_codes::USAGE_ERROR => Self::UsageError,
            exit_codes::TRANSIENT_INTERMITTENT => Self::IntermittentFailure,
            exit_codes::TIMEOUT => Self::Timeout,
            other => Self::Custom(other),
        }
    }

    pub fn from_error(err: &crate::fault::CliExecutionError) -> Self {
        match err {
            crate::fault::CliExecutionError::ParseError { code, .. } => Self::from_raw(*code),
            crate::fault::CliExecutionError::NonZeroExit { code, .. } => Self::from_raw(*code),
            crate::fault::CliExecutionError::SocketTimeout
            | crate::fault::CliExecutionError::ProcessTimeout
            | crate::fault::CliExecutionError::TransientIo(_)
            | crate::fault::CliExecutionError::ConnectionReset
            | crate::fault::CliExecutionError::BrokenPipe
            | crate::fault::CliExecutionError::LockContention(_) => Self::IntermittentFailure,
            crate::fault::CliExecutionError::Panic(_)
            | crate::fault::CliExecutionError::MaxRetriesExceeded(_)
            | crate::fault::CliExecutionError::Other(_) => Self::GeneralFailure,
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    pub fn is_intermittent(&self) -> bool {
        matches!(self, Self::IntermittentFailure | Self::Timeout)
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Inspect task, memory, or workflow DAG details
    Inspect(InspectArgs),
    /// Manage project-scoped persona memory
    Memory(MemoryArgs),
    /// Replay a task from a specific checkpoint step or list its recorded checkpoints
    Replay(ReplayArgs),
    /// Fork an existing task from a checkpoint into a new independent Task
    Fork(ForkArgs),
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
    /// Manage multi-agent subagent rosters (agents/*.md) and dynamic coordination
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

#[derive(Subcommand, Debug, Clone)]
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

#[derive(Subcommand, Debug, Clone)]
pub enum RigAction {
    /// Boot all persistent seats defined in the subagent roster (agents/*.md or legacy YAML)
    Up {
        #[arg(short, long, default_value = "agents")]
        spec: PathBuf,
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
    /// Shutdown and cleanup all seats of a Rig
    Down {
        #[arg(short, long, default_value = "agents")]
        spec: PathBuf,
    },
    /// Inspect health and status of all seats in a Rig
    Status {
        #[arg(short, long, default_value = "agents")]
        spec: PathBuf,
    },
    /// Deliver a high-level goal to the Kernel Operator for autonomous team dispatch
    Run {
        #[arg(short, long, default_value = "agents")]
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
        #[arg(short, long, default_value = "agents")]
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
    /// Coordinate 1-4 teammate seats with anti-cycle and turn-depth guards (OpenMausBot style)
    Coordinate(crate::commands::rig::RigCoordinateArgs),
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

#[derive(
    clap::ValueEnum,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_exit_code_and_intermittent_catching() {
        assert!(SafeExitCode::Success.is_success());
        assert_eq!(SafeExitCode::from_raw(2).as_i32(), exit_codes::USAGE_ERROR);
        assert!(SafeExitCode::from_raw(75).is_intermittent());
        assert!(Cli::is_intermittent_exit_code(2));
        assert_eq!(
            Cli::sanitize_exit_code(2),
            exit_codes::TRANSIENT_INTERMITTENT
        );
        assert_eq!(Cli::sanitize_exit_code(0), exit_codes::SUCCESS);

        let err_res: Result<(), _> = Err("network timed out during dispatch");
        let caught = Cli::catch_intermittent_error(err_res);
        assert_eq!(caught, Err(crate::fault::CliExecutionError::SocketTimeout));

        let code_from_err =
            SafeExitCode::from_error(&crate::fault::CliExecutionError::SocketTimeout);
        assert_eq!(code_from_err, SafeExitCode::IntermittentFailure);
    }

    #[test]
    fn test_bounded_cli_execution_buffer() {
        let mut buf = CliExecutionBuffer::new(3);
        assert_eq!(buf.capacity(), 3);
        assert!(buf.is_empty());

        assert_eq!(buf.push("cmd1"), None);
        assert_eq!(buf.push("cmd2"), None);
        assert_eq!(buf.push("cmd3"), None);
        assert!(buf.is_full());
        assert_eq!(buf.len(), 3);

        // FIFO eviction on overflow
        let evicted = buf.push("cmd4");
        assert_eq!(evicted, Some("cmd1"));
        assert_eq!(buf.dropped_count(), 1);
        assert_eq!(buf.len(), 3);

        let items = buf.drain_all();
        assert_eq!(items, vec!["cmd2", "cmd3", "cmd4"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_cli_execution_buffer_pop_returns_oldest_item_and_none_when_empty() {
        let mut buf = CliExecutionBuffer::new(2);
        assert_eq!(buf.pop(), None);

        buf.push("first");
        buf.push("second");
        assert_eq!(buf.pop(), Some("first"));
        assert_eq!(buf.pop(), Some("second"));
        assert_eq!(buf.pop(), None);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_cli_execution_buffer_clear_empties_items_without_resetting_drop_count() {
        let mut buf = CliExecutionBuffer::new(1);
        buf.push("kept until overflow");
        buf.push("evicts first");
        assert_eq!(buf.dropped_count(), 1);

        buf.clear();

        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.dropped_count(), 1);
        assert_eq!(buf.pop(), None);
    }

    #[test]
    fn test_try_parse_safe_returns_structured_error_for_test_harness_arguments() {
        // The test process arguments are not an agent-ctl command, so safe parsing
        // should report a structured parse error instead of exiting the process.
        let result = Cli::try_parse_safe();

        assert!(matches!(
            result,
            Err(crate::fault::CliExecutionError::ParseError { .. })
        ));
    }

    #[test]
    fn test_cli_validate() {
        let empty_run = Cli {
            short_version: None,
            command: Commands::Run {
                agent_id: String::new(),
                goal: "do something".into(),
                workspace: ".".into(),
                engine: String::new(),
                sandbox: None,
                persona_path: None,
                timeout: None,
                auto_approve: true,
            },
        };
        assert!(empty_run.validate().is_err());

        let valid_run = Cli {
            short_version: None,
            command: Commands::Run {
                agent_id: "agent-1".into(),
                goal: "do something".into(),
                workspace: ".".into(),
                engine: String::new(),
                sandbox: None,
                persona_path: None,
                timeout: None,
                auto_approve: true,
            },
        };
        assert!(valid_run.validate().is_ok());
    }
}
