use agent_domain::VerificationSpec;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_runtime::{AutoJudge, AutoJudgeConfig, TournamentCandidate, TournamentExecutor};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct WorkspaceWritingEngine {
    kind: EngineKind,
    files_to_write: Vec<(String, String)>,
    stdout: String,
    exit_code: i32,
    duration_ms: u128,
    should_fail: Option<String>,
}

#[async_trait]
impl ComputeEngine for WorkspaceWritingEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        if let Some(ref err) = self.should_fail {
            return Err(EngineError::ProcessFailed(err.clone()));
        }
        let base = PathBuf::from(&input.working_dir);
        let _ = tokio::fs::create_dir_all(&base).await;
        for (rel_path, content) in &self.files_to_write {
            let full = base.join(rel_path);
            if let Some(parent) = full.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            tokio::fs::write(&full, content)
                .await
                .map_err(|e| EngineError::ProcessFailed(format!("write failed: {e}")))?;
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

#[tokio::test]
async fn tdd_best_of_3_tournament_selects_highest_verified_engine() {
    let base_dir = std::env::temp_dir().join(format!(
        "tuzygent_tournament_tdd_{}",
        uuid::Uuid::new_v4()
    ));
    let ws_agy = base_dir.join("agy");
    let ws_claude = base_dir.join("claude");
    let ws_codex = base_dir.join("codex");

    // Candidate 1 (Antigravity): writes valid JSON and passes all 3 verification specs
    let engine_agy = Arc::new(WorkspaceWritingEngine {
        kind: EngineKind::Antigravity,
        files_to_write: vec![
            ("report.json".into(), r#"{"status": "verified", "score": 100}"#.into()),
            ("SOLUTION.md".into(), "STATUS: PASS\nClean patch.".into()),
        ],
        stdout: "Antigravity completed all checks".into(),
        exit_code: 0,
        duration_ms: 420,
        should_fail: None,
    });
    // Candidate 2 (Claude): writes valid JSON, but missing pattern in SOLUTION.md (passes 2/3 specs)
    let engine_claude = Arc::new(WorkspaceWritingEngine {
        kind: EngineKind::ClaudeCode,
        files_to_write: vec![
            ("report.json".into(), r#"{"status": "partial"}"#.into()),
            ("SOLUTION.md".into(), "STATUS: INCOMPLETE".into()),
        ],
        stdout: "Claude completed partially".into(),
        exit_code: 0,
        duration_ms: 250,
        should_fail: None,
    });
    // Candidate 3 (Codex): writes corrupt JSON (passes only 1/3 specs)
    let engine_codex = Arc::new(WorkspaceWritingEngine {
        kind: EngineKind::Codex,
        files_to_write: vec![
            ("report.json".into(), "{corrupt json".into()),
            ("SOLUTION.md".into(), "STATUS: FAIL".into()),
        ],
        stdout: "Codex finished with invalid json".into(),
        exit_code: 0,
        duration_ms: 180,
        should_fail: None,
    });

    let specs = vec![
        VerificationSpec::FileExists {
            path: "report.json".into(),
            min_bytes: Some(5),
        },
        VerificationSpec::JsonValid {
            path: "report.json".into(),
        },
        VerificationSpec::ContentContains {
            path: "SOLUTION.md".into(),
            pattern: "STATUS: PASS".into(),
        },
    ];

    let candidates = vec![
        TournamentCandidate::new("candidate-codex", engine_codex)
            .with_workspace_override(&ws_codex)
            .with_diff_bytes_hint(512),
        TournamentCandidate::new("candidate-claude", engine_claude)
            .with_workspace_override(&ws_claude)
            .with_diff_bytes_hint(384),
        TournamentCandidate::new("candidate-antigravity", engine_agy)
            .with_workspace_override(&ws_agy)
            .with_diff_bytes_hint(256),
    ];

    let executor = TournamentExecutor::with_named_candidates(&base_dir, candidates)
        .with_verification_specs(specs);
    let outcome = executor
        .execute_tournament(&EngineInput::new("generate verified report", base_dir.to_string_lossy()))
        .await
        .expect("tournament should select winner");

    assert_eq!(outcome.winner_id, "candidate-antigravity");
    assert_eq!(outcome.winner_engine, EngineKind::Antigravity);
    assert!(!outcome.used_fallback);
    assert_eq!(outcome.scorecards.len(), 3);

    // Verify ranking order in scorecards: Antigravity (3/3) > Claude (2/3) > Codex (1/3)
    assert_eq!(outcome.scorecards[0].candidate_id, "candidate-antigravity");
    assert_eq!(outcome.scorecards[0].passed_verifications, 3);
    assert!(outcome.scorecards[0].succeeded);
    assert_eq!(outcome.scorecards[1].candidate_id, "candidate-claude");
    assert_eq!(outcome.scorecards[1].passed_verifications, 2);
    assert!(!outcome.scorecards[1].succeeded);
    assert_eq!(outcome.scorecards[2].candidate_id, "candidate-codex");
    assert_eq!(outcome.scorecards[2].passed_verifications, 1);
    assert!(!outcome.scorecards[2].succeeded);
    let _ = tokio::fs::remove_dir_all(&base_dir).await;
}

#[tokio::test]
async fn tdd_tournament_tie_breaker_prefers_lower_latency_and_smaller_diff() {
    let base_dir = std::env::temp_dir().join(format!("tuzygent_tiebreak_{}", uuid::Uuid::new_v4()));
    let _ = tokio::fs::create_dir_all(&base_dir).await;
    let marker = base_dir.join("ok.txt");
    tokio::fs::write(&marker, "verified content").await.expect("write marker");

    // All 3 pass the exact same verification spec, but differ in speed and diff size
    let slow_heavy = Arc::new(WorkspaceWritingEngine {
        kind: EngineKind::Codex,
        files_to_write: vec![],
        stdout: "patch A".into(),
        exit_code: 0,
        duration_ms: 4000,
        should_fail: None,
    });
    let medium_compact = Arc::new(WorkspaceWritingEngine {
        kind: EngineKind::ClaudeCode,
        files_to_write: vec![],
        stdout: "patch B".into(),
        exit_code: 0,
        duration_ms: 300,
        should_fail: None,
    });
    let slow_medium = Arc::new(WorkspaceWritingEngine {
        kind: EngineKind::Antigravity,
        files_to_write: vec![],
        stdout: "patch C".into(),
        exit_code: 0,
        duration_ms: 2500,
        should_fail: None,
    });

    let candidates = vec![
        TournamentCandidate::new("codex-slow-heavy", slow_heavy).with_diff_bytes_hint(16 * 1024),
        TournamentCandidate::new("claude-fast-compact", medium_compact).with_diff_bytes_hint(256),
        TournamentCandidate::new("agy-medium", slow_medium).with_diff_bytes_hint(4 * 1024),
    ];
    let specs = vec![VerificationSpec::FileExists {
        path: "ok.txt".into(),
        min_bytes: Some(5),
    }];
    let judge = AutoJudge::new(AutoJudgeConfig {
        verification_weight: 100.0,
        exit_code_zero_bonus: 50.0,
        non_empty_output_bonus: 10.0,
        latency_penalty_per_sec: 4.0,
        diff_size_penalty_per_kb: 2.0,
        min_output_bytes: 1,
    });

    let outcome = TournamentExecutor::with_named_candidates(&base_dir, candidates)
        .with_verification_specs(specs)
        .with_judge(judge)
        .execute_tournament(&EngineInput::new("optimize hot path", base_dir.to_string_lossy()))
        .await
        .expect("tournament should succeed");

    assert_eq!(outcome.winner_id, "claude-fast-compact");
    assert_eq!(outcome.winner_engine, EngineKind::ClaudeCode);
    assert_eq!(outcome.scorecards[0].candidate_id, "claude-fast-compact");
    assert_eq!(outcome.scorecards[1].candidate_id, "agy-medium");
    assert_eq!(outcome.scorecards[2].candidate_id, "codex-slow-heavy");
    let _ = tokio::fs::remove_dir_all(&base_dir).await;
}

#[tokio::test]
async fn tdd_tournament_partial_failure_survives_and_full_failure_triggers_fallback() {
    let invocation_counter = Arc::new(AtomicUsize::new(0));

    struct CountingFallbackEngine(Arc<AtomicUsize>);
    #[async_trait]
    impl ComputeEngine for CountingFallbackEngine {
        async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(EngineOutput {
                session_id: None,
                stdout_content: "fallback succeeded".into(),
                exit_code: 0,
                execution_duration_ms: 50,
                engine_used: EngineKind::Mock,
            })
        }
        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }

    // Case 1: 2 candidates fail, 1 succeeds -> Fallback should NOT be called
    let partial_candidates = vec![
        TournamentCandidate::new(
            "failed-1",
            Arc::new(WorkspaceWritingEngine {
                kind: EngineKind::Antigravity,
                files_to_write: vec![],
                stdout: String::new(),
                exit_code: 1,
                duration_ms: 10,
                should_fail: Some("connection reset".into()),
            }),
        ),
        TournamentCandidate::new(
            "survivor-claude",
            Arc::new(WorkspaceWritingEngine {
                kind: EngineKind::ClaudeCode,
                files_to_write: vec![],
                stdout: "survivor output".into(),
                exit_code: 0,
                duration_ms: 120,
                should_fail: None,
            }),
        ),
        TournamentCandidate::new(
            "failed-2",
            Arc::new(WorkspaceWritingEngine {
                kind: EngineKind::Codex,
                files_to_write: vec![],
                stdout: String::new(),
                exit_code: 1,
                duration_ms: 10,
                should_fail: Some("quota exceeded".into()),
            }),
        ),
    ];

    let executor_partial = TournamentExecutor::with_named_candidates("/tmp", partial_candidates)
        .with_fallback_engine(Arc::new(CountingFallbackEngine(invocation_counter.clone())));
    let outcome_partial = executor_partial
        .execute_tournament(&EngineInput::new("task", "/tmp"))
        .await
        .expect("survivor should win");
    assert_eq!(outcome_partial.winner_id, "survivor-claude");
    assert!(!outcome_partial.used_fallback);
    assert_eq!(invocation_counter.load(Ordering::SeqCst), 0);

    // Also verify ComputeEngine trait implementation on TournamentExecutor
    let compute_out = ComputeEngine::execute(&executor_partial, &EngineInput::new("task", "/tmp"))
        .await
        .expect("ComputeEngine trait on TournamentExecutor should succeed");
    assert_eq!(compute_out.stdout_content, "survivor output");

    // Case 2: All candidates fail -> Fallback MUST be called and rescue execution
    let all_failed_candidates = vec![
        TournamentCandidate::new(
            "failed-a",
            Arc::new(WorkspaceWritingEngine {
                kind: EngineKind::Antigravity,
                files_to_write: vec![],
                stdout: String::new(),
                exit_code: 1,
                duration_ms: 10,
                should_fail: Some("crash A".into()),
            }),
        ),
        TournamentCandidate::new(
            "failed-b",
            Arc::new(WorkspaceWritingEngine {
                kind: EngineKind::ClaudeCode,
                files_to_write: vec![],
                stdout: String::new(),
                exit_code: 1,
                duration_ms: 10,
                should_fail: Some("crash B".into()),
            }),
        ),
    ];

    let executor_fallback = TournamentExecutor::with_named_candidates("/tmp", all_failed_candidates)
        .with_fallback_engine(Arc::new(CountingFallbackEngine(invocation_counter.clone())));
    let outcome_fb = executor_fallback
        .execute_tournament(&EngineInput::new("task", "/tmp"))
        .await
        .expect("fallback should rescue when all candidates fail");
    assert!(outcome_fb.used_fallback);
    assert_eq!(outcome_fb.winner_id, "fallback");
    assert_eq!(outcome_fb.winner_output.stdout_content, "fallback succeeded");
    assert_eq!(invocation_counter.load(Ordering::SeqCst), 1);

    // Case 3: All candidates fail and no fallback configured -> Returns Err
    let no_fb = vec![TournamentCandidate::new(
        "only-fail",
        Arc::new(WorkspaceWritingEngine {
            kind: EngineKind::Codex,
            files_to_write: vec![],
            stdout: String::new(),
            exit_code: 1,
            duration_ms: 10,
            should_fail: Some("total outage".into()),
        }),
    )];
    let err_res = TournamentExecutor::with_named_candidates("/tmp", no_fb)
        .execute_tournament(&EngineInput::new("task", "/tmp"))
        .await;
    assert!(err_res.is_err());
}
