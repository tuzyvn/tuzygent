use super::*;
use agent_engine::EngineKind;
use async_trait::async_trait;
use std::sync::atomic::AtomicUsize;

struct RecordingEngine(Mutex<Option<EngineInput>>);

#[async_trait]
impl ComputeEngine for RecordingEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let mut guard = self.0.lock().map_err(|e| EngineError::ProcessFailed(format!("Lock error: {e}")))?;
        *guard = Some(input.clone());
        Ok(EngineOutput {
            stdout_content: "ok".into(),
            exit_code: 0,
            execution_duration_ms: 0,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }
    fn engine_kind(&self) -> EngineKind { EngineKind::Mock }
}

struct ChaosTestEngine {
    invocations: Arc<AtomicUsize>,
    panics_first: usize,
    fails_first: usize,
}

impl ChaosTestEngine {
    fn new() -> Self {
        Self { invocations: Arc::new(AtomicUsize::new(0)), panics_first: 0, fails_first: 0 }
    }
}

#[async_trait]
impl ComputeEngine for ChaosTestEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let count = self.invocations.fetch_add(1, Ordering::SeqCst);
        if count < self.panics_first { panic!("intentional chaos test panic"); }
        if count < self.fails_first { return Err(EngineError::ProcessFailed(format!("chaos err {count}"))); }
        Ok(EngineOutput {
            stdout_content: format!("executed: {}", input.prompt_or_command),
            exit_code: 0,
            execution_duration_ms: 1,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }
    fn engine_kind(&self) -> EngineKind { EngineKind::Mock }
}

#[tokio::test]
async fn execution_applies_sandbox_and_output_limit() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Arc::new(RecordingEngine(Mutex::new(None)));
    let service = ExecutionService::new("/tmp", None, None);
    service.execute(engine.clone(), &EngineInput::new("hello", "/tmp")).await?;
    let guard = engine.0.lock().map_err(|e| format!("Lock error: {e}"))?;
    let input = guard.as_ref().ok_or("Recorded input is missing")?;
    assert!(input.command_sandbox.is_some());
    assert_eq!(input.max_output_bytes, Some(service.guardrail().max_output_bytes));
    Ok(())
}

#[tokio::test]
async fn policy_rejects_before_engine_is_called() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Arc::new(RecordingEngine(Mutex::new(None)));
    let service = ExecutionService::new("/tmp", None, None);
    assert!(matches!(
        service.execute(engine.clone(), &EngineInput::new("rm -rf /", "/tmp")).await,
        Err(EngineError::PolicyRejected(_))
    ));
    let guard = engine.0.lock().map_err(|e| format!("Lock error: {e}"))?;
    assert!(guard.is_none());
    Ok(())
}

#[tokio::test]
async fn recording_engine_handles_poisoned_mutex_gracefully() {
    let engine = Arc::new(RecordingEngine(Mutex::new(None)));
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = engine.0.lock();
        std::panic::resume_unwind(Box::new("intentional poison"));
    }));
    let result = engine.execute(&EngineInput::new("test", "/tmp")).await;
    assert!(matches!(result, Err(EngineError::ProcessFailed(_))));
}

#[tokio::test]
async fn execution_fault_injection_and_retry_recovery() {
    let engine = Arc::new(ChaosTestEngine::new());
    let policy = ExecutionFaultPolicy::intermittent(ExecutionFault::SocketTimeout, 2).with_max_injections(2);
    let service = ExecutionService::new("/tmp", None, None).with_fault_policy(policy).with_max_retries(3);
    let res = service.execute(engine.clone(), &EngineInput::new("echo ok", "/tmp")).await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn execution_panic_isolation_and_recovery() {
    let mut eng = ChaosTestEngine::new();
    eng.panics_first = 1;
    let engine = Arc::new(eng);
    let service = ExecutionService::new("/tmp", None, None).with_max_retries(2);
    let res = service.execute(engine.clone(), &EngineInput::new("echo safe", "/tmp")).await;
    assert!(res.is_ok());
    assert_eq!(engine.invocations.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn execution_retry_transient_failure_recovery() {
    let mut eng = ChaosTestEngine::new();
    eng.fails_first = 2;
    let engine = Arc::new(eng);
    let service = ExecutionService::new("/tmp", None, None).with_max_retries(3);
    let res = service.execute_resilient(engine.clone(), &EngineInput::new("echo retry", "/tmp")).await;
    assert!(res.is_ok());
    assert_eq!(engine.invocations.load(Ordering::SeqCst), 3);
}
