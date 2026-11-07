use super::*;
use agent_engine::EngineKind;
use async_trait::async_trait;
use std::sync::atomic::AtomicUsize;

struct RecordingEngine(Mutex<Option<EngineInput>>);

#[async_trait]
impl ComputeEngine for RecordingEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let mut guard = self
            .0
            .lock()
            .map_err(|e| EngineError::ProcessFailed(format!("Lock error: {e}")))?;
        *guard = Some(input.clone());
        Ok(EngineOutput {
            stdout_content: "ok".into(),
            exit_code: 0,
            execution_duration_ms: 0,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

struct ChaosTestEngine {
    invocations: Arc<AtomicUsize>,
    panics_first: usize,
    fails_first: usize,
}

impl ChaosTestEngine {
    fn new() -> Self {
        Self {
            invocations: Arc::new(AtomicUsize::new(0)),
            panics_first: 0,
            fails_first: 0,
        }
    }
}

#[async_trait]
impl ComputeEngine for ChaosTestEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let count = self.invocations.fetch_add(1, Ordering::SeqCst);
        if count < self.panics_first {
            std::panic::resume_unwind(Box::new("intentional chaos test panic"));
        }
        if count < self.fails_first {
            return Err(EngineError::ProcessFailed(format!("chaos err {count}")));
        }
        Ok(EngineOutput {
            stdout_content: format!("executed: {}", input.prompt_or_command),
            exit_code: 0,
            execution_duration_ms: 1,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

#[tokio::test]
async fn execution_applies_sandbox_and_output_limit() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Arc::new(RecordingEngine(Mutex::new(None)));
    let service = ExecutionService::new("/tmp", None, None);
    service
        .execute(engine.clone(), &EngineInput::new("hello", "/tmp"))
        .await?;
    let guard = engine.0.lock().map_err(|e| format!("Lock error: {e}"))?;
    let input = guard.as_ref().ok_or("Recorded input is missing")?;
    assert!(input.command_sandbox.is_some());
    assert_eq!(
        input.max_output_bytes,
        Some(service.guardrail().max_output_bytes)
    );
    Ok(())
}

#[tokio::test]
async fn policy_rejects_before_engine_is_called() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Arc::new(RecordingEngine(Mutex::new(None)));
    let service = ExecutionService::new("/tmp", None, None);
    assert!(matches!(
        service
            .execute(engine.clone(), &EngineInput::new("rm -rf /", "/tmp"))
            .await,
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
    let policy =
        ExecutionFaultPolicy::intermittent(ExecutionFault::SocketTimeout, 2).with_max_injections(2);
    let service = ExecutionService::new("/tmp", None, None)
        .with_fault_policy(policy)
        .with_max_retries(3);
    let res = service
        .execute(engine.clone(), &EngineInput::new("echo ok", "/tmp"))
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn execution_panic_isolation_and_recovery() {
    let mut eng = ChaosTestEngine::new();
    eng.panics_first = 1;
    let engine = Arc::new(eng);
    let service = ExecutionService::new("/tmp", None, None).with_max_retries(2);
    let res = service
        .execute(engine.clone(), &EngineInput::new("echo safe", "/tmp"))
        .await;
    assert!(res.is_ok());
    assert_eq!(engine.invocations.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn execution_retry_transient_failure_recovery() {
    let mut eng = ChaosTestEngine::new();
    eng.fails_first = 2;
    let engine = Arc::new(eng);
    let service = ExecutionService::new("/tmp", None, None).with_max_retries(3);
    let res = service
        .execute_resilient(engine.clone(), &EngineInput::new("echo retry", "/tmp"))
        .await;
    assert!(res.is_ok());
    assert_eq!(engine.invocations.load(Ordering::SeqCst), 3);
}

struct PanickingPollFuture;
impl Future for PanickingPollFuture {
    type Output = String;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::resume_unwind(Box::new("worker poll panic triggered"));
    }
}

struct NonUnpinTestFuture {
    value: i32,
    _marker: std::marker::PhantomPinned,
}
impl Future for NonUnpinTestFuture {
    type Output = i32;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.value)
    }
}

#[tokio::test]
async fn catch_unwind_future_catches_poll_panic_safely() {
    let fut = CatchUnwindFuture::new(PanickingPollFuture);
    let res = fut.await;
    assert!(res.is_err(), "Expected panic to be caught as Err");
    let err = res.err().unwrap();
    let msg = err
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| err.downcast_ref::<String>().map(|s| s.as_str()));
    assert_eq!(
        msg,
        Some("worker poll panic triggered"),
        "Unexpected panic payload type"
    );
}

#[tokio::test]
async fn catch_unwind_future_projects_non_unpin_future() {
    let fut = CatchUnwindFuture::new(NonUnpinTestFuture {
        value: 42,
        _marker: std::marker::PhantomPinned,
    });
    // Pinned to box to poll !Unpin future
    let res = Box::pin(fut).await;
    assert!(matches!(res, Ok(42)));
}

#[tokio::test]
async fn catch_unwind_future_handles_pending_and_ready() {
    use std::sync::atomic::AtomicU8;
    struct StepFuture(Arc<AtomicU8>);
    impl Future for StepFuture {
        type Output = &'static str;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let state = self.0.fetch_add(1, Ordering::SeqCst);
            if state == 0 {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready("completed")
            }
        }
    }

    let state = Arc::new(AtomicU8::new(0));
    let fut = CatchUnwindFuture::new(StepFuture(state.clone()));
    let res = fut.await;
    assert_eq!(res.unwrap(), "completed");
    assert_eq!(state.load(Ordering::SeqCst), 2);
}

// ==============================================================================
// Additional Rigorous Verification Tests for CatchUnwindFuture
// Covering: Panic-catching, Cancellation, and Drop Safety
// ==============================================================================

struct DropTracker {
    dropped: Arc<AtomicBool>,
    drop_count: Arc<AtomicUsize>,
}

impl DropTracker {
    fn new() -> (Self, Arc<AtomicBool>, Arc<AtomicUsize>) {
        let dropped = Arc::new(AtomicBool::new(false));
        let drop_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                dropped: dropped.clone(),
                drop_count: drop_count.clone(),
            },
            dropped,
            drop_count,
        )
    }
}

impl Drop for DropTracker {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        self.drop_count.fetch_add(1, Ordering::SeqCst);
    }
}

struct DelayedPanicFuture {
    polled: Arc<AtomicUsize>,
}

impl Future for DelayedPanicFuture {
    type Output = &'static str;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let count = self.polled.fetch_add(1, Ordering::SeqCst);
        if count == 0 {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            std::panic::resume_unwind(Box::new("delayed worker panic after pending state"));
        }
    }
}

#[tokio::test]
async fn catch_unwind_future_catches_delayed_panic_after_pending() {
    let polled = Arc::new(AtomicUsize::new(0));
    let fut = CatchUnwindFuture::new(DelayedPanicFuture {
        polled: polled.clone(),
    });
    let res = fut.await;
    assert!(res.is_err(), "Expected delayed panic to be caught as Err");
    assert_eq!(polled.load(Ordering::SeqCst), 2);
    let err = res.err().unwrap();
    let msg = err
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| err.downcast_ref::<String>().map(|s| s.as_str()));
    assert_eq!(
        msg,
        Some("delayed worker panic after pending state"),
        "Unexpected panic payload type"
    );
}

struct CustomPayloadPanicFuture;
impl Future for CustomPayloadPanicFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::panic_any(9999_u64);
    }
}

#[tokio::test]
async fn catch_unwind_future_catches_arbitrary_panic_payload() {
    let fut = CatchUnwindFuture::new(CustomPayloadPanicFuture);
    let res = fut.await;
    assert!(res.is_err(), "Expected panic to be caught as Err");
    let err = res.err().unwrap();
    let val = err
        .downcast_ref::<u64>()
        .expect("Expected u64 panic payload");
    assert_eq!(*val, 9999_u64);
}

struct CancellablePendingFuture {
    polled: Arc<AtomicBool>,
    _tracker: DropTracker,
}

impl Future for CancellablePendingFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.polled.store(true, Ordering::SeqCst);
        Poll::Pending
    }
}

#[tokio::test]
async fn catch_unwind_future_cancellation_select_drops_inner_safely() {
    let polled = Arc::new(AtomicBool::new(false));
    let (tracker, dropped, drop_count) = DropTracker::new();

    let fut = CatchUnwindFuture::new(CancellablePendingFuture {
        polled: polled.clone(),
        _tracker: tracker,
    });

    let resolved = tokio::select! {
        _ = fut => true,
        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => false,
    };
    assert!(!resolved, "CatchUnwindFuture should not have resolved");

    assert!(
        polled.load(Ordering::SeqCst),
        "Future should have been polled before cancellation"
    );
    assert!(
        dropped.load(Ordering::SeqCst),
        "Inner future must be dropped on cancellation"
    );
    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        1,
        "Inner future drop must occur exactly once"
    );
}

#[tokio::test]
async fn catch_unwind_future_cancellation_timeout_drops_inner_safely() {
    let polled = Arc::new(AtomicBool::new(false));
    let (tracker, dropped, drop_count) = DropTracker::new();

    let fut = CatchUnwindFuture::new(CancellablePendingFuture {
        polled: polled.clone(),
        _tracker: tracker,
    });

    let timeout_res = tokio::time::timeout(std::time::Duration::from_millis(10), fut).await;
    assert!(timeout_res.is_err(), "Expected timeout cancellation");
    assert!(
        polled.load(Ordering::SeqCst),
        "Future should have been polled before timeout"
    );
    assert!(
        dropped.load(Ordering::SeqCst),
        "Inner future must be dropped upon timeout cancellation"
    );
    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        1,
        "Drop count must be exactly once"
    );
}

struct PanickingWithDropFuture {
    _tracker: DropTracker,
}

impl Future for PanickingWithDropFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::resume_unwind(Box::new("intentional panic during poll with drop tracking"));
    }
}

#[tokio::test]
async fn catch_unwind_future_drop_safety_after_panic() {
    let (tracker, dropped, drop_count) = DropTracker::new();
    let fut = CatchUnwindFuture::new(PanickingWithDropFuture { _tracker: tracker });

    let res = fut.await;
    assert!(res.is_err(), "Expected panic to be caught as Err");
    // CatchUnwindFuture has been consumed and dropped by `fut.await`
    assert!(
        dropped.load(Ordering::SeqCst),
        "Inner future must be dropped cleanly after panic"
    );
    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        1,
        "Drop destructor must be executed exactly once"
    );
}

#[tokio::test]
async fn catch_unwind_future_drop_safety_unpolled() {
    let (tracker, dropped, drop_count) = DropTracker::new();
    let fut = CatchUnwindFuture::new(CancellablePendingFuture {
        polled: Arc::new(AtomicBool::new(false)),
        _tracker: tracker,
    });

    drop(fut);

    assert!(
        dropped.load(Ordering::SeqCst),
        "Unpolled future must be dropped cleanly"
    );
    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        1,
        "Drop destructor must run exactly once"
    );
}

struct CompletingWithDropFuture {
    value: i32,
    _tracker: DropTracker,
}

impl Future for CompletingWithDropFuture {
    type Output = i32;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.value)
    }
}

#[tokio::test]
async fn catch_unwind_future_drop_safety_on_normal_completion() {
    let (tracker, dropped, drop_count) = DropTracker::new();
    let fut = CatchUnwindFuture::new(CompletingWithDropFuture {
        value: 100,
        _tracker: tracker,
    });

    let res = fut.await;
    assert_eq!(res.unwrap(), 100);
    assert!(
        dropped.load(Ordering::SeqCst),
        "Inner future must be dropped on normal completion"
    );
    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        1,
        "Drop destructor must run exactly once"
    );
}

struct NonUnpinDropFuture {
    _marker: std::marker::PhantomPinned,
    _tracker: DropTracker,
}

impl Future for NonUnpinDropFuture {
    type Output = &'static str;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

#[tokio::test]
async fn catch_unwind_future_drop_safety_non_unpin_pinned() {
    let (tracker, dropped, drop_count) = DropTracker::new();
    let fut = CatchUnwindFuture::new(NonUnpinDropFuture {
        _marker: std::marker::PhantomPinned,
        _tracker: tracker,
    });

    {
        let mut pinned = Box::pin(fut);
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let poll_res = pinned.as_mut().poll(&mut cx);
        assert!(poll_res.is_pending());
        // pinned goes out of scope here and is dropped
    }

    assert!(
        dropped.load(Ordering::SeqCst),
        "Pinned !Unpin inner future must be dropped"
    );
    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        1,
        "Drop count must be exactly 1"
    );
}
