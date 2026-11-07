use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    pub call_count: AtomicUsize,
    pub callback: F,
}

impl<F> CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    pub fn new(callback: F) -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            callback,
        }
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.call_count.store(0, Ordering::SeqCst);
    }
}

type BoxedEngineFn =
    Box<dyn Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync>;

impl CallbackMockEngine<BoxedEngineFn> {
    pub fn with_constant_output(output_text: impl Into<String>) -> Self {
        let text = output_text.into();
        Self::new(Box::new(move |_input, _idx| {
            Ok(EngineOutput {
                stdout_content: text.clone(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }))
    }

    pub fn with_timeout() -> Self {
        Self::new(Box::new(move |_input, _idx| Err(EngineError::Timeout)))
    }

    pub fn with_failure(msg: impl Into<String>) -> Self {
        let message = msg.into();
        Self::new(Box::new(move |_input, _idx| {
            Err(EngineError::ProcessFailed(message.clone()))
        }))
    }

    pub fn with_error(msg: impl Into<String>) -> Self {
        Self::with_failure(msg)
    }

    pub fn with_exit_code(code: i32, stdout: impl Into<String>) -> Self {
        let text = stdout.into();
        Self::new(Box::new(move |_input, _idx| {
            Ok(EngineOutput {
                stdout_content: text.clone(),
                exit_code: code,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }))
    }

    pub fn with_sequence(outputs: Vec<Result<EngineOutput, EngineError>>) -> Self {
        let list = std::sync::Mutex::new(outputs);
        Self::new(Box::new(move |_input, idx| {
            let mut guard = list.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_empty() {
                Ok(EngineOutput {
                    stdout_content: format!("sequence exhausted at {}", idx),
                    exit_code: 0,
                    execution_duration_ms: 5,
                    engine_used: EngineKind::Mock,
                    session_id: None,
                })
            } else {
                guard.remove(0)
            }
        }))
    }
}

#[async_trait]
impl<F> ComputeEngine for CallbackMockEngine<F>
where
    F: Fn(&EngineInput, usize) -> Result<EngineOutput, EngineError> + Send + Sync,
{
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);
        (self.callback)(input, count)
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}
