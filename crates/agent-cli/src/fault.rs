use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Structured execution error capturing CLI parse issues, non-zero exit codes, and transient failures
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CliExecutionError {
    #[error("CLI argument parse error (exit code {code}): {message}")]
    ParseError { code: i32, message: String },
    #[error("Socket timeout during CLI operation")]
    SocketTimeout,
    #[error("CLI process execution timed out")]
    ProcessTimeout,
    #[error("Transient I/O failure during CLI operation: {0}")]
    TransientIo(String),
    #[error("Connection reset by peer during CLI operation")]
    ConnectionReset,
    #[error("Broken pipe communicating with CLI process or daemon")]
    BrokenPipe,
    #[error("Lock contention during CLI execution: {0}")]
    LockContention(String),
    #[error("CLI command or mission exited with non-zero code {code}: {message}")]
    NonZeroExit { code: i32, message: String },
    #[error("CLI operation panicked: {0}")]
    Panic(String),
    #[error("Max retry attempts ({0}) exceeded during CLI operation")]
    MaxRetriesExceeded(usize),
    #[error("CLI execution error: {0}")]
    Other(String),
}

impl From<std::io::Error> for CliExecutionError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::TimedOut => CliExecutionError::SocketTimeout,
            std::io::ErrorKind::ConnectionReset => CliExecutionError::ConnectionReset,
            std::io::ErrorKind::BrokenPipe => CliExecutionError::BrokenPipe,
            _ => CliExecutionError::TransientIo(err.to_string()),
        }
    }
}

impl From<clap::Error> for CliExecutionError {
    fn from(err: clap::Error) -> Self {
        CliExecutionError::ParseError {
            code: err.exit_code(),
            message: err.to_string(),
        }
    }
}

impl From<anyhow::Error> for CliExecutionError {
    fn from(err: anyhow::Error) -> Self {
        let msg = err.to_string();
        let lower = msg.to_lowercase();
        if lower.contains("timed out") || lower.contains("timeout") {
            CliExecutionError::SocketTimeout
        } else if lower.contains("connection reset") {
            CliExecutionError::ConnectionReset
        } else if lower.contains("broken pipe") {
            CliExecutionError::BrokenPipe
        } else if lower.contains("lock") || lower.contains("busy") || lower.contains("contention") {
            CliExecutionError::LockContention(msg)
        } else if lower.contains("i/o failure") || lower.contains("io failure") || lower.contains("transient io") {
            CliExecutionError::TransientIo(msg)
        } else if lower.contains("non-zero code 2")
            || lower.contains("exit code 2")
            || lower.contains("exit status: 2")
            || lower.contains("exited with code 2")
            || lower.contains("status: 2")
        {
            CliExecutionError::NonZeroExit {
                code: 2,
                message: msg,
            }
        } else if lower.contains("non-zero code")
            || lower.contains("exit code")
            || lower.contains("exit status")
            || lower.contains("status: exit")
        {
            CliExecutionError::NonZeroExit {
                code: 1,
                message: msg,
            }
        } else {
            CliExecutionError::Other(msg)
        }
    }
}

/// Simulated faults for CLI chaos engineering and resilience testing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliFault {
    SocketTimeout,
    ProcessTimeout,
    TransientIo,
    ConnectionReset,
    BrokenPipe,
    LockContention,
    NonZeroExit(i32),
    Panic,
}

impl CliFault {
    pub fn to_execution_error(&self) -> CliExecutionError {
        match self {
            CliFault::SocketTimeout => CliExecutionError::SocketTimeout,
            CliFault::ProcessTimeout => CliExecutionError::ProcessTimeout,
            CliFault::TransientIo => {
                CliExecutionError::TransientIo("Simulated transient I/O failure".to_string())
            }
            CliFault::ConnectionReset => CliExecutionError::ConnectionReset,
            CliFault::BrokenPipe => CliExecutionError::BrokenPipe,
            CliFault::LockContention => {
                CliExecutionError::LockContention("Simulated lock acquisition contention".to_string())
            }
            CliFault::NonZeroExit(code) => CliExecutionError::NonZeroExit {
                code: *code,
                message: format!("Mission exited with non-zero code {}", code),
            },
            CliFault::Panic => {
                CliExecutionError::Panic("Simulated panic inside CLI dispatch handler".to_string())
            }
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            CliFault::SocketTimeout
            | CliFault::ProcessTimeout
            | CliFault::TransientIo
            | CliFault::ConnectionReset
            | CliFault::BrokenPipe
            | CliFault::LockContention
            | CliFault::Panic => true,
            CliFault::NonZeroExit(code) => *code == 2 || *code == 1,
        }
    }
}

/// Simulates a CLI fault by converting it directly into a CliExecutionError
pub fn simulate_cli_fault(fault: &CliFault) -> Result<(), CliExecutionError> {
    Err(fault.to_execution_error())
}

/// Policy governing CLI fault injection frequency, counts, and intervals
#[derive(Debug, Clone, Default)]
pub struct CliFaultPolicy {
    fault: Option<CliFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl CliFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: CliFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: CliFault, interval: u32) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: Some(interval.max(1)),
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn with_max_injections(mut self, max: usize) -> Self {
        self.max_injections = Some(max);
        self
    }

    pub fn should_inject(&mut self) -> Option<CliFault> {
        let fault = self.fault.as_ref()?;
        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);
        let inject = match self.fail_interval {
            Some(interval) if interval > 0 => (self.invocation_count - 1).is_multiple_of(interval),
            _ => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}

/// Determines whether a CliExecutionError is transient and eligible for resilient retry
pub fn is_retryable_cli_error(err: &CliExecutionError) -> bool {
    match err {
        CliExecutionError::SocketTimeout
        | CliExecutionError::ProcessTimeout
        | CliExecutionError::TransientIo(_)
        | CliExecutionError::ConnectionReset
        | CliExecutionError::BrokenPipe
        | CliExecutionError::LockContention(_)
        | CliExecutionError::Panic(_) => true,
        CliExecutionError::NonZeroExit { code, .. } => *code == 2 || *code == 1,
        CliExecutionError::ParseError { .. }
        | CliExecutionError::MaxRetriesExceeded(_)
        | CliExecutionError::Other(_) => false,
    }
}
