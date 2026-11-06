use agent_engine::EngineError;
use serde::{Deserialize, Serialize};

/// Known fault injection categories for chaos engineering in agent execution
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    ServiceUnavailable,
    WorkerCrash,
    CapacityOverload,
    CorruptedOutput,
    Panic,
}

impl ExecutionFault {
    pub fn to_engine_error(&self) -> EngineError {
        match self {
            Self::SocketTimeout => EngineError::Timeout,
            Self::ConnectionReset => {
                EngineError::ProcessFailed("Connection reset by peer during engine execution".to_string())
            }
            Self::TransientIo => {
                EngineError::ProcessFailed("Transient I/O failure during execution: broken pipe".to_string())
            }
            Self::ServiceUnavailable => {
                EngineError::ApiRequestFailed("503 Service Unavailable: execution backend temporarily busy".to_string())
            }
            Self::WorkerCrash => {
                EngineError::ProcessFailed("Execution worker process crashed (exit code 137 / SIGKILL)".to_string())
            }
            Self::CapacityOverload => {
                EngineError::ProcessFailed("Execution capacity overload: worker queue full".to_string())
            }
            Self::CorruptedOutput => {
                EngineError::ProcessFailed("Corrupted execution output stream: invalid payload received".to_string())
            }
            Self::Panic => {
                EngineError::ProcessFailed("Execution worker panicked unexpectedly".to_string())
            }
        }
    }

    pub fn is_transient(&self) -> bool {
        match self {
            Self::SocketTimeout
            | Self::ConnectionReset
            | Self::TransientIo
            | Self::ServiceUnavailable
            | Self::WorkerCrash
            | Self::CapacityOverload => true,
            Self::CorruptedOutput | Self::Panic => false,
        }
    }

    pub fn simulate_fault(&self) -> Result<(), EngineError> {
        Err(self.to_engine_error())
    }
}

/// Helper function to directly simulate an execution fault
pub fn simulate_execution_fault(fault: &ExecutionFault) -> Result<(), EngineError> {
    fault.simulate_fault()
}

/// Policy governing fault injection frequency, counts, and intervals
#[derive(Debug, Clone, Default)]
pub struct ExecutionFaultPolicy {
    fault: Option<ExecutionFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl ExecutionFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: ExecutionFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: ExecutionFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<ExecutionFault> {
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

/// Determines whether an EngineError is transient and should trigger retry recovery
pub fn is_retryable_execution_error(err: &EngineError) -> bool {
    match err {
        EngineError::PolicyRejected(_) => false,
        EngineError::ProcessFailed(msg) if msg.contains("canceled") => false,
        _ => true,
    }
}
