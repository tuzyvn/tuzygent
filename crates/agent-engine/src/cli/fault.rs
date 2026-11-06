use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::traits::EngineError;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    #[error("Socket timeout communicating with CLI process or daemon")]
    SocketTimeout,
    #[error("CLI process execution timed out")]
    ProcessTimeout,
    #[error("Broken pipe or connection reset communicating with CLI process")]
    BrokenPipe,
    #[error("Transient I/O failure during CLI operation: {0}")]
    TransientIo(String),
    #[error("CLI process execution failed: {0}")]
    ProcessFailed(String),
    #[error("Corrupted or malformed CLI output: {0}")]
    CorruptedOutput(String),
    #[error("CLI binary not found or unreachable: {0}")]
    CliNotFound(String),
    #[error("Max retry attempts ({0}) exceeded during CLI recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliFault {
    SocketTimeout,
    ProcessTimeout,
    BrokenPipe,
    TransientIo,
    ProcessFailed,
    CorruptedOutput,
    CliNotFound,
}

impl CliFault {
    pub fn to_cli_error(&self) -> CliError {
        match self {
            CliFault::SocketTimeout => CliError::SocketTimeout,
            CliFault::ProcessTimeout => CliError::ProcessTimeout,
            CliFault::BrokenPipe => CliError::BrokenPipe,
            CliFault::TransientIo => CliError::TransientIo(
                "Simulated transient I/O failure during CLI process execution".to_string(),
            ),
            CliFault::ProcessFailed => CliError::ProcessFailed(
                "Simulated CLI process failure with non-zero exit code".to_string(),
            ),
            CliFault::CorruptedOutput => {
                CliError::CorruptedOutput("Corrupted or malformed CLI stream output".to_string())
            }
            CliFault::CliNotFound => {
                CliError::CliNotFound("Simulated CLI binary not found in PATH".to_string())
            }
        }
    }

    pub fn to_engine_error(&self) -> EngineError {
        match self {
            CliFault::SocketTimeout | CliFault::ProcessTimeout => EngineError::Timeout,
            CliFault::CliNotFound => EngineError::CliNotFound(self.to_cli_error().to_string()),
            _ => EngineError::ProcessFailed(self.to_cli_error().to_string()),
        }
    }

    pub fn simulate_fault(&self) -> Result<(), CliError> {
        Err(self.to_cli_error())
    }
}

pub fn simulate_cli_fault(fault: &CliFault) -> Result<(), CliError> {
    fault.simulate_fault()
}

pub fn simulate_fault(fault: &CliFault) -> Result<(), CliError> {
    fault.simulate_fault()
}

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
            Some(interval) => (self.invocation_count - 1).is_multiple_of(interval),
            None => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn fault(&self) -> Option<&CliFault> {
        self.fault.as_ref()
    }
}

pub fn is_retryable_cli_error_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("reset by peer")
        || lower.contains("transient")
        || lower.contains("temporarily unavailable")
        || lower.contains("resource temporarily unavailable")
        || lower.contains("os error 32") // EPIPE
        || lower.contains("os error 11") // EAGAIN
        || lower.contains("os error 104") // ECONNRESET
        || lower.contains("interrupted system call")
        || lower.contains("i/o error")
        || lower.contains("io error")
}

pub fn is_retryable_engine_error(err: &EngineError) -> bool {
    match err {
        EngineError::Timeout => true,
        EngineError::ProcessFailed(msg) => is_retryable_cli_error_message(msg),
        _ => false,
    }
}
