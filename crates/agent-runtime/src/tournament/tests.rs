use super::*;
use agent_domain::{VerificationResult, VerificationSpec};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

struct SimulatedEngine {
    kind: EngineKind,
    stdout: String,
    exit_code: i32,
    duration_ms: u128,
    fail_with: Option<String>,
}

impl SimulatedEngine {
    fn success(kind: EngineKind, stdout: &str, duration_ms: u128) -> Arc<Self> {
        Arc::new(Self {
            kind,
            stdout: stdout.to_string(),
            exit_code: 0,
            duration_ms,
            fail_with: None,
        })
    }

    fn non_zero(kind: EngineKind, stdout: &str, exit_code: i32, duration_ms: u128) -> Arc<Self> {
        Arc::new(Self {
            kind,
            stdout: stdout.to_string(),
            exit_code,
            duration_ms,
            fail_with: None,
        })
    }

    fn failing(kind: EngineKind, err_msg: &str) -> Arc<Self> {
        Arc::new(Self {
            kind,
            stdout: String::new(),
            exit_code: 1,
            duration_ms: 0,
            fail_with: Some(err_msg.to_string()),
        })
    }
}

#[async_trait]
impl ComputeEngine for SimulatedEngine {
    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        if let Some(ref msg) = self.fail_with {
            return Err(EngineError::ApiRequestFailed(msg.clone()));
        }
        Ok(EngineOutput {
            session_id: None,
            stdout_content: self.stdout.clone(),
            exit_code: self.exit_code,
            execution_duration_ms: self.duration_ms,
            engine_used: self.kind.clone(),
        })
    }

    fn engine_kind(&self) -> EngineKind {
        self.kind.clone()
    }
}

fn dummy_verification(passed: bool) -> VerificationResult {
    VerificationResult {
        passed,
        spec: VerificationSpec::Custom {
            description: "unit check".into(),
        },
        details: if passed { "ok".into() } else { "failed".into() },
        executed_at: chrono::Utc::now(),
    }
}

#[test]
fn auto_judge_scores_verification_pass_rate_and_penalties_accurately() {
    let judge = AutoJudge::default();

    let out_fast_clean = Ok(EngineOutput {
        session_id: None,
        stdout_content: "fn add(a: i32, b: i32) -> i32 { a + b }".into(),
        exit_code: 0,
        execution_duration_ms: 500,
        engine_used: EngineKind::ClaudeCode,
    });
    let out_slow_bloated = Ok(EngineOutput {
        session_id: None,
        stdout_content: "x".repeat(10 * 1024),
        exit_code: 0,
        execution_duration_ms: 5000,
        engine_used: EngineKind::Codex,
    });
    let out_partial_verif = Ok(EngineOutput {
        session_id: None,
        stdout_content: "partial".into(),
        exit_code: 0,
        execution_duration_ms: 400,
        engine_used: EngineKind::Antigravity,
    });

    let sc_fast = judge.score_candidate(
        "claude",
        EngineKind::ClaudeCode,
        &out_fast_clean,
        vec![dummy_verification(true), dummy_verification(true)],
        128,
    );
    let sc_slow = judge.score_candidate(
        "codex",
        EngineKind::Codex,
        &out_slow_bloated,
        vec![dummy_verification(true), dummy_verification(true)],
        10 * 1024,
    );
    let sc_partial = judge.score_candidate(
        "antigravity",
        EngineKind::Antigravity,
        &out_partial_verif,
        vec![dummy_verification(true), dummy_verification(false)],
        128,
    );

    assert!(sc_fast.succeeded);
    assert!(sc_slow.succeeded);
    assert!(!sc_partial.succeeded);

    assert!(
        sc_fast.total_score > sc_slow.total_score,
        "Faster & smaller diff candidate ({}) should outscore slower & bloated candidate ({})",
        sc_fast.total_score,
        sc_slow.total_score
    );
    assert!(
        sc_slow.total_score > sc_partial.total_score,
        "100% verified candidate ({}) should outscore 50% verified candidate ({})",
        sc_slow.total_score,
        sc_partial.total_score
    );
}

#[test]
fn auto_judge_assigns_negative_floor_score_to_errored_candidate() {
    let judge = AutoJudge::default();
    let err_out: Result<EngineOutput, EngineError> =
        Err(EngineError::Timeout);
    let sc = judge.score_candidate("timed-out", EngineKind::Codex, &err_out, vec![], 0);
    assert!(!sc.succeeded);
    assert!(sc.total_score <= -1000.0);
    assert!(sc.error_message.is_some());
}

#[tokio::test]
async fn tournament_executes_candidates_in_parallel_and_picks_best() {
    struct BarrierEngine {
        kind: EngineKind,
        active: Arc<AtomicUsize>,
        max_seen: Arc<AtomicUsize>,
        barrier: Arc<tokio::sync::Barrier>,
        duration_ms: u128,
    }

    #[async_trait]
    impl ComputeEngine for BarrierEngine {
        async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
            let cur = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(cur, Ordering::SeqCst);
            self.barrier.wait().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(EngineOutput {
                session_id: None,
                stdout_content: format!("output from {}", self.kind),
                exit_code: 0,
                execution_duration_ms: self.duration_ms,
                engine_used: self.kind.clone(),
            })
        }

        fn engine_kind(&self) -> EngineKind {
            self.kind.clone()
        }
    }

    let active = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(tokio::sync::Barrier::new(3));

    let e1: Arc<dyn ComputeEngine> = Arc::new(BarrierEngine {
        kind: EngineKind::Antigravity,
        active: active.clone(),
        max_seen: max_seen.clone(),
        barrier: barrier.clone(),
        duration_ms: 800,
    });
    let e2: Arc<dyn ComputeEngine> = Arc::new(BarrierEngine {
        kind: EngineKind::ClaudeCode,
        active: active.clone(),
        max_seen: max_seen.clone(),
        barrier: barrier.clone(),
        duration_ms: 150,
    });
    let e3: Arc<dyn ComputeEngine> = Arc::new(BarrierEngine {
        kind: EngineKind::Codex,
        active: active.clone(),
        max_seen: max_seen.clone(),
        barrier: barrier.clone(),
        duration_ms: 1200,
    });

    let executor = TournamentExecutor::new("/tmp", vec![e1, e2, e3]);
    let outcome = executor
        .execute_tournament(&EngineInput::new("solve algorithm", "/tmp"))
        .await
        .expect("tournament should succeed");

    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        3,
        "All 3 candidate engines must execute concurrently in parallel"
    );
    assert_eq!(outcome.winner_engine, EngineKind::ClaudeCode);
    assert!(!outcome.used_fallback);
    assert_eq!(outcome.scorecards.len(), 3);
}

#[tokio::test]
async fn tournament_uses_fallback_engine_when_all_candidates_fail() {
    let candidates = vec![
        TournamentCandidate::new(
            "agy-primary",
            SimulatedEngine::failing(EngineKind::Antigravity, "rate limit 429"),
        ),
        TournamentCandidate::new(
            "claude-primary",
            SimulatedEngine::failing(EngineKind::ClaudeCode, "upstream 503"),
        ),
        TournamentCandidate::new(
            "codex-primary",
            SimulatedEngine::non_zero(EngineKind::Codex, "compilation error", 1, 300),
        ),
    ];
    let fallback = SimulatedEngine::success(
        EngineKind::Mock,
        "fallback recovery output",
        100,
    );

    let executor = TournamentExecutor::with_named_candidates("/tmp", candidates)
        .with_fallback_engine(fallback);
    let outcome = executor
        .execute_tournament(&EngineInput::new("implement feature", "/tmp"))
        .await
        .expect("fallback engine should rescue tournament");

    assert!(outcome.used_fallback);
    assert_eq!(outcome.winner_id, "fallback");
    assert_eq!(outcome.winner_engine, EngineKind::Mock);
    assert_eq!(outcome.winner_output.stdout_content, "fallback recovery output");
    assert_eq!(outcome.scorecards.len(), 4);
}
