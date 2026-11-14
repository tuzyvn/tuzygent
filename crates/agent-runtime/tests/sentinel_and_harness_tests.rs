use agent_runtime::{
    classify_shell_error, failure_signature, normalize_signature_text, ExecutionLoopConfig,
    ExecutionLoopGuard, FailureSignature, HarnessMemoryStore, LoopVerdict, RepeatDetector,
    RepeatObservation,
};

#[test]
fn normalize_signature_text_clusters_volatile_details_and_preserves_apostrophes() {
    let a = normalize_signature_text(
        "can't read \"/tmp/run-42/data.json\" for bx_9f2c81 from https://api.local/v1?x=1 by alice@acme.org (550e8400-e29b-41d4-a716-446655440000) step 17",
    );
    let b = normalize_signature_text(
        "can't read \"/var/tmp/other.txt\" for bx_11aa99 from http://other.host/z by bob@acme.org (123e4567-e89b-12d3-a456-426614174000) step 999",
    );
    assert_eq!(a, b);
    assert!(a.contains("can't"));
    assert!(a.contains("<q>"));
    assert!(a.contains("<id>"));
    assert!(a.contains("<url>"));
    assert!(a.contains("<email>"));
    assert!(a.contains("<n>"));

    let path_case = normalize_signature_text("failed opening /home/fox/tuzygent/foo.rs at line 12");
    assert_eq!(path_case, "failed opening <path> at line <n>");
}

#[test]
fn classify_shell_error_and_failure_signature_work_deterministically() {
    assert_eq!(
        classify_shell_error("command timed out after 30s (exit code 124)"),
        (None, "timed out")
    );
    assert_eq!(
        classify_shell_error("Permission denied (EACCES) exit status: 13"),
        (Some(13), "permission denied")
    );

    let s1 = failure_signature(
        "tool_error",
        "bash",
        "ENOENT: /tmp/a/file1.txt exited with code 2",
    );
    let s2 = failure_signature(
        "tool_error",
        "bash",
        "ENOENT: /tmp/b/file2.txt exit code: 2",
    );
    assert_eq!(s1.signature, s2.signature);
    assert!(s1.signature.starts_with("tool_error:"));
    assert_eq!(s1.title, "exit 2: no such file");

    let json = serde_json::to_string(&s1).unwrap();
    let decoded: FailureSignature = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, s1);
}

#[test]
fn repeat_detector_and_execution_loop_guard_enforce_caps() {
    assert_eq!(RepeatDetector::canonical_tool_key("bash", "   "), None);
    assert_eq!(RepeatDetector::canonical_tool_key("bash", "bash"), None);
    let k = RepeatDetector::canonical_tool_key("bash", "ls   -la  /tmp").unwrap();
    assert_eq!(k, "bash:ls -la /tmp");

    let mut detector = RepeatDetector::new(vec![3, 2], 2);
    assert_eq!(
        detector.record("t1", "k1"),
        RepeatObservation {
            count: 1,
            threshold_hit: None
        }
    );
    assert_eq!(
        detector.record("t1", "k1"),
        RepeatObservation {
            count: 2,
            threshold_hit: Some(2)
        }
    );
    // bounded LRU eviction keeps capacity <= 2
    detector.record("t1", "k2");
    detector.record("t1", "k3");
    assert_eq!(detector.record("t1", "k1").count, 1);
    detector.settle("t1");
    assert_eq!(detector.record("t1", "k3").count, 1);

    let obs_json = serde_json::to_string(&RepeatObservation {
        count: 2,
        threshold_hit: Some(2),
    })
    .unwrap();
    let _: RepeatObservation = serde_json::from_str(&obs_json).unwrap();

    let cfg = ExecutionLoopConfig {
        max_total_turns: 2,
        repeat_warn_at: 2,
        repeat_break_at: 3,
        consecutive_error_break_at: 3,
    };
    let cfg_json = serde_json::to_string(&cfg).unwrap();
    let cfg_decoded: ExecutionLoopConfig = serde_json::from_str(&cfg_json).unwrap();
    assert_eq!(cfg_decoded, cfg);
    let _ = ExecutionLoopConfig::default();

    let mut guard = ExecutionLoopGuard::new(cfg);
    assert!(matches!(guard.step_turn(), LoopVerdict::Allow { count: 1 }));
    assert!(matches!(guard.step_turn(), LoopVerdict::Allow { count: 2 }));
    assert!(matches!(guard.step_turn(), LoopVerdict::Break { count: 3, .. }));

    assert!(matches!(
        guard.observe_tool_call("s1", "bash", "cat foo"),
        LoopVerdict::Allow { count: 1 }
    ));
    assert!(matches!(
        guard.observe_tool_call("s1", "bash", "cat foo"),
        LoopVerdict::Warn { count: 2, .. }
    ));
    assert!(matches!(
        guard.observe_tool_call("s1", "bash", "cat foo"),
        LoopVerdict::Break { count: 3, .. }
    ));

    let v1 = guard.observe_failure("s1", "tool", "read_file", "failed on /tmp/run1/a.txt step 1");
    let v2 = guard.observe_failure("s1", "tool", "read_file", "failed on /tmp/run2/b.txt step 2");
    let v3 = guard.observe_failure("s1", "tool", "read_file", "failed on /tmp/run3/c.txt step 3");
    assert!(matches!(v1, LoopVerdict::Allow { count: 1 }));
    assert!(matches!(v2, LoopVerdict::Warn { count: 2, .. }));
    assert!(matches!(v3, LoopVerdict::Break { count: 3, .. }));

    let v_json = serde_json::to_string(&v3).unwrap();
    let v_dec: LoopVerdict = serde_json::from_str(&v_json).unwrap();
    assert_eq!(v_dec, v3);

    guard.settle_scope("s1");
    assert!(matches!(
        guard.observe_failure("s1", "tool", "read_file", "failed on /tmp/run4/d.txt step 4"),
        LoopVerdict::Allow { count: 1 }
    ));
}

#[tokio::test]
async fn harness_memory_store_deduplicates_normalized_error_patterns_in_memory_and_disk(
) -> anyhow::Result<()> {
    let mem_store = HarnessMemoryStore::memory();
    let first = mem_store
        .record_lesson(
            "cargo_test",
            "agent-runtime",
            "Missing file /tmp/run-101/config.toml at line 42",
            "Create default config",
        )
        .await?;
    let second = mem_store
        .record_lesson(
            "cargo_test",
            "agent-runtime",
            "Missing file /var/data/run-999/other.toml at line 88",
            "Create default config",
        )
        .await?;
    assert_eq!(first.id, second.id);
    assert_eq!(second.success_count, 2);

    let temp_dir = std::env::temp_dir().join(format!(
        "harness_norm_dedup_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let disk_store = HarnessMemoryStore::new(temp_dir.to_str().unwrap());
    let d1 = disk_store
        .record_lesson(
            "shell",
            "worker-1",
            "Error opening /tmp/job-1/out.log exit 1",
            "Ensure dir exists",
        )
        .await?;
    let d2 = disk_store
        .record_lesson(
            "shell",
            "worker-1",
            "Error opening /tmp/job-99/alt.log exit 1",
            "Ensure dir exists",
        )
        .await?;
    assert_eq!(d1.id, d2.id);
    assert_eq!(d2.success_count, 2);
    assert_eq!(disk_store.load_lessons().await.len(), 1);

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    Ok(())
}
