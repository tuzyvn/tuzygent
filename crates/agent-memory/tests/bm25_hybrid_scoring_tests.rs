use agent_memory::{
    compute_bm25_score, compute_bm25_score_with_params, compute_bm25_term_score, compute_idf,
    compute_recency_weight, rank_lesson_blocks, rank_structured_lessons, score_lesson_hybrid,
    score_lesson_relevance, Bm25Params, DurableMemoryStore, MemoryService,
    ResilientDurableMemoryStore, StructuredLesson,
};
use chrono::{Duration, Utc};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(prefix: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("bm25_hybrid_test_{}_{}_{}", prefix, nanos, id));
        std::fs::create_dir_all(&dir).expect("failed to create temporary test directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_bm25_term_frequency_monotonicity_and_saturation() {
    let params = Bm25Params::default();
    let idf = compute_idf(1, 10);
    let doc_len = 20;
    let avg_doc_len = 20.0;

    let score_tf0 = compute_bm25_term_score(0.0, doc_len, avg_doc_len, idf, params);
    let score_tf1 = compute_bm25_term_score(1.0, doc_len, avg_doc_len, idf, params);
    let score_tf2 = compute_bm25_term_score(2.0, doc_len, avg_doc_len, idf, params);
    let score_tf4 = compute_bm25_term_score(4.0, doc_len, avg_doc_len, idf, params);
    let score_tf10 = compute_bm25_term_score(10.0, doc_len, avg_doc_len, idf, params);

    assert_eq!(score_tf0, 0.0, "Zero term frequency must yield 0.0 score");
    assert!(
        score_tf1 > score_tf0
            && score_tf2 > score_tf1
            && score_tf4 > score_tf2
            && score_tf10 > score_tf4,
        "BM25 score must strictly increase with term frequency: tf1={score_tf1}, tf2={score_tf2}, tf4={score_tf4}, tf10={score_tf10}"
    );

    // Saturation property: marginal gain from tf=1->2 must exceed gain from tf=4->5
    let score_tf5 = compute_bm25_term_score(5.0, doc_len, avg_doc_len, idf, params);
    let gain_1_to_2 = score_tf2 - score_tf1;
    let gain_4_to_5 = score_tf5 - score_tf4;
    assert!(
        gain_1_to_2 > gain_4_to_5,
        "BM25 k1 saturation must produce diminishing marginal returns: gain_1_2={gain_1_to_2}, gain_4_5={gain_4_to_5}"
    );

    // Upper bound asymptotic limit: idf * (k1 + 1.0)
    let asymptotic_upper_bound = idf * (params.k1 + 1.0);
    assert!(
        score_tf10 < asymptotic_upper_bound,
        "BM25 score must remain bounded by idf * (k1 + 1)"
    );

    // End-to-end document level TF comparison with equal document length
    let doc_single_match = "tokio runtime channel worker queue alpha beta gamma delta epsilon";
    let doc_triple_match = "tokio runtime tokio worker tokio alpha beta gamma delta epsilon";
    let corpus = [doc_single_match, doc_triple_match];
    let s1 = compute_bm25_score("tokio", doc_single_match, &corpus);
    let s3 = compute_bm25_score("tokio", doc_triple_match, &corpus);
    assert!(
        s3 > s1,
        "Document with 3 occurrences of 'tokio' ({s3}) must outscore equal-length document with 1 occurrence ({s1})"
    );
}

#[test]
fn test_bm25_document_length_normalization() {
    let concise_doc = "Fix redis cluster failover timeout";
    let verbose_noisy_doc = "Fix redis issue during routine maintenance window when multiple background cron jobs and telemetry collectors print verbose diagnostic logs across staging nodes";
    let corpus = [concise_doc, verbose_noisy_doc];

    let score_concise = compute_bm25_score("redis", concise_doc, &corpus);
    let score_verbose = compute_bm25_score("redis", verbose_noisy_doc, &corpus);

    assert!(
        score_concise > score_verbose,
        "Concise document ({score_concise}) with same TF=1 must outscore verbose noisy document ({score_verbose})"
    );

    // Verify that setting b = 0.0 disables document length penalty
    let no_len_norm = Bm25Params { k1: 1.5, b: 0.0 };
    let score_concise_b0 =
        compute_bm25_score_with_params("redis", concise_doc, &corpus, no_len_norm);
    let score_verbose_b0 =
        compute_bm25_score_with_params("redis", verbose_noisy_doc, &corpus, no_len_norm);
    assert!(
        (score_concise_b0 - score_verbose_b0).abs() < 1e-9,
        "When b=0.0, documents with identical TF must receive equal BM25 scores"
    );
}

#[test]
fn test_bm25_inverse_document_frequency_specificity() {
    let total_docs = 25;
    let idf_df1 = compute_idf(1, total_docs);
    let idf_df5 = compute_idf(5, total_docs);
    let idf_df15 = compute_idf(15, total_docs);
    let idf_df25 = compute_idf(25, total_docs);

    assert!(
        idf_df1 > idf_df5 && idf_df5 > idf_df15 && idf_df15 > idf_df25 && idf_df25 > 0.0,
        "IDF must strictly decrease as document frequency increases while staying positive"
    );

    // Corpus where 'service' and 'error' appear almost everywhere, but 'ebpf_verifier' appears once
    let corpus = vec![
        "service error in ebpf_verifier kernel probe load",
        "service error in http gateway router",
        "service error in background cron worker",
        "service error in metrics exporter",
        "service error in database connection pool",
        "service error in authentication middleware",
    ];

    let doc_rare_match = "kernel probe ebpf_verifier check";
    let doc_common_match = "general backend service check";
    let mut full_corpus = corpus.clone();
    full_corpus.push(doc_rare_match);
    full_corpus.push(doc_common_match);

    let query = "service ebpf_verifier";
    let score_rare = compute_bm25_score(query, doc_rare_match, &full_corpus);
    let score_common = compute_bm25_score(query, doc_common_match, &full_corpus);

    assert!(
        score_rare > score_common * 1.5,
        "Document matching rare high-IDF term 'ebpf_verifier' ({score_rare}) must significantly outscore document matching common low-IDF term 'service' ({score_common})"
    );
}

#[test]
fn test_hybrid_semantic_scoring_coverage_confidence_and_time_decay() {
    let now = Utc::now();

    // 1. Multi-token semantic coverage boost vs single-token repetition
    let full_coverage_lesson = StructuredLesson {
        id: "les-cov-full".to_string(),
        timestamp: now.to_rfc3339(),
        task_title: "Resolve sqlite wal checkpoint lock".to_string(),
        trigger_error: Some("database table locked during wal checkpoint".to_string()),
        remediation_applied: Some("Enable busy_timeout and passive checkpoint".to_string()),
        general_insight: "Configure sqlite wal mode with busy_timeout".to_string(),
        success: true,
    };

    let single_term_repeated_lesson = StructuredLesson {
        id: "les-cov-single".to_string(),
        timestamp: now.to_rfc3339(),
        task_title: "Configure sqlite driver for sqlite storage".to_string(),
        trigger_error: Some("sqlite connection string invalid in sqlite config".to_string()),
        remediation_applied: Some("Update sqlite path in sqlite env".to_string()),
        general_insight: "Always validate sqlite file path".to_string(),
        success: true,
    };

    let corpus = vec![
        full_coverage_lesson.clone(),
        single_term_repeated_lesson.clone(),
    ];
    let query = "sqlite wal checkpoint busy_timeout";

    let b_full = score_lesson_hybrid(query, &full_coverage_lesson, &corpus, now);
    let b_single = score_lesson_hybrid(query, &single_term_repeated_lesson, &corpus, now);

    assert_eq!(b_full.coverage_ratio, 1.0);
    assert_eq!(b_single.coverage_ratio, 0.25);
    assert!(
        b_full.final_score > b_single.final_score,
        "Lesson covering all 4 semantic query concepts ({}) must outscore lesson repeating 1 concept ({})",
        b_full.final_score,
        b_single.final_score
    );

    // 2. Confidence weight (verified success vs unverified failure)
    let mut failed_lesson = full_coverage_lesson.clone();
    failed_lesson.id = "les-failed".to_string();
    failed_lesson.success = false;
    failed_lesson.remediation_applied = None;

    let score_verified = score_lesson_relevance(query, &full_coverage_lesson, now);
    let score_failed = score_lesson_relevance(query, &failed_lesson, now);
    assert!(
        score_verified > score_failed * 2.0,
        "Verified successful lesson ({score_verified}) must outscore failed lesson without remediation ({score_failed})"
    );

    // 3. Time-decay (recency weighting)
    let mut lesson_30d_old = full_coverage_lesson.clone();
    lesson_30d_old.id = "les-30d".to_string();
    lesson_30d_old.timestamp = (now - Duration::days(30)).to_rfc3339();

    let mut lesson_180d_old = full_coverage_lesson.clone();
    lesson_180d_old.id = "les-180d".to_string();
    lesson_180d_old.timestamp = (now - Duration::days(180)).to_rfc3339();

    let score_now = score_lesson_relevance(query, &full_coverage_lesson, now);
    let score_30d = score_lesson_relevance(query, &lesson_30d_old, now);
    let score_180d = score_lesson_relevance(query, &lesson_180d_old, now);

    assert!(
        score_now > score_30d && score_30d > score_180d,
        "Time-decay must rank newer lessons higher when relevance is otherwise equal: now={score_now}, 30d={score_30d}, 180d={score_180d}"
    );
    assert!(
        compute_recency_weight(&now.to_rfc3339(), now)
            > compute_recency_weight(&(now - Duration::days(60)).to_rfc3339(), now)
    );

    // 4. Unrelated query returns 0.0
    assert_eq!(
        score_lesson_relevance(
            "kubernetes ingress tls certificate",
            &full_coverage_lesson,
            now
        ),
        0.0
    );
}

#[tokio::test]
async fn test_durable_memory_hybrid_retrieval_with_noisy_records() {
    let ws = TempWorkspace::new("durable_noisy");
    let mut resilient_store = ResilientDurableMemoryStore::open(ws.path());
    let now = Utc::now();

    let lessons = vec![
        // Noise 1: Completely unrelated lesson
        StructuredLesson {
            id: "noise-ui".to_string(),
            timestamp: now.to_rfc3339(),
            task_title: "Update CSS theme colors in dashboard".to_string(),
            trigger_error: None,
            remediation_applied: None,
            general_insight: "Use CSS variables for dark mode toggle".to_string(),
            success: true,
        },
        // Noise 2: Verbose lesson matching only generic term 'timeout'
        StructuredLesson {
            id: "noise-generic-timeout".to_string(),
            timestamp: now.to_rfc3339(),
            task_title: "Investigate intermittent timeout in CI pipeline".to_string(),
            trigger_error: Some("Step timed out waiting for artifact upload".to_string()),
            remediation_applied: Some("Increase runner timeout setting".to_string()),
            general_insight:
                "Large debug binaries need longer upload window in slow runners during peak hours"
                    .to_string(),
            success: true,
        },
        // Stale Failed Match: Matches 'quorum' and 'raft', but old (150 days ago) and failed
        StructuredLesson {
            id: "stale-failed-raft".to_string(),
            timestamp: (now - Duration::days(150)).to_rfc3339(),
            task_title: "Debug raft quorum election timeout".to_string(),
            trigger_error: Some("raft leader lost quorum during partition".to_string()),
            remediation_applied: None,
            general_insight: "Attempted manual restart but split_brain persisted".to_string(),
            success: false,
        },
        // Target Best Match: Recent, verified, concise, matches rare terms 'raft', 'quorum', 'split_brain', 'timeout'
        StructuredLesson {
            id: "best-verified-raft".to_string(),
            timestamp: (now - Duration::hours(2)).to_rfc3339(),
            task_title: "Fix raft quorum split_brain election timeout".to_string(),
            trigger_error: Some("raft split_brain detected after network timeout".to_string()),
            remediation_applied: Some(
                "Enable pre_vote and check_quorum in raft config".to_string(),
            ),
            general_insight:
                "Pre-vote prevents disruptive elections and resolves raft quorum split_brain"
                    .to_string(),
            success: true,
        },
    ];

    for lesson in &lessons {
        resilient_store
            .save_structured_lesson(lesson)
            .await
            .expect("failed to save structured lesson");
    }

    let query = "raft quorum split_brain timeout";
    let ranked = resilient_store
        .retrieve_ranked_lessons(query, 5)
        .await
        .expect("failed to retrieve ranked lessons");

    // Unrelated 'noise-ui' must be excluded (score == 0.0)
    assert_eq!(ranked.len(), 3, "Only matching lessons should be returned");
    assert_eq!(
        ranked[0].0.id, "best-verified-raft",
        "Recent verified multi-keyword lesson must rank #1"
    );
    assert_eq!(
        ranked[1].0.id, "stale-failed-raft",
        "High-IDF multi-keyword match (even older/failed) must rank #2 above single common keyword match"
    );
    assert_eq!(
        ranked[2].0.id, "noise-generic-timeout",
        "Single common keyword match must rank last among matches"
    );
    assert!(ranked[0].1 > ranked[1].1 && ranked[1].1 > ranked[2].1);

    // Verify load_relevant_context orders the best match first under High Relevance section
    let context = resilient_store
        .load_relevant_context(query)
        .await
        .expect("failed to load relevant context");
    assert!(context.contains("## High Relevance to Current Goal:"));
    let best_pos = context
        .find("Fix raft quorum split_brain election timeout")
        .expect("best lesson must appear in context");
    let generic_pos = context
        .find("Investigate intermittent timeout in CI pipeline")
        .expect("generic match should appear later in context");
    assert!(
        best_pos < generic_pos,
        "Highest-ranked BM25+Hybrid lesson must appear before lower-ranked lessons in prompt context"
    );

    // Also verify rank_structured_lessons and rank_lesson_blocks directly
    let direct_ranked = rank_structured_lessons(query, &lessons, now);
    assert_eq!(direct_ranked[0].0.id, "best-verified-raft");

    let raw_blocks = vec![
        format!("- [Learned {}] [FAILURE] Goal 'Old': raft timeout", (now - Duration::days(90)).to_rfc3339()),
        format!("- [Learned {}] [SUCCESS] Goal 'New': raft quorum split_brain timeout solved by pre_vote", now.to_rfc3339()),
    ];
    let ranked_blocks = rank_lesson_blocks(query, &raw_blocks, now);
    assert_eq!(ranked_blocks.len(), 2);
    assert!(ranked_blocks[0].0.contains("New"));
}

#[test]
fn test_persona_memory_service_fts5_bm25_and_hybrid_ranking() {
    let ws = TempWorkspace::new("persona_bm25");
    let store =
        MemoryService::open(ws.path(), "proj-alpha", "sre-agent").expect("failed to open store");

    let ep = store
        .record_episode("task-1", "all verification checks passed", true)
        .expect("failed to record episode");

    // Record 1: Matches only 1 keyword in content, verbose text
    let id_weak = store
        .propose_lesson(
            "When investigating general database connection latency, check disk IOPS and network packet drops across availability zones",
            "general maintenance",
            &ep,
        )
        .expect("failed to propose weak lesson");
    store
        .activate(&id_weak)
        .expect("failed to activate weak lesson");

    // Record 2: Matches multiple query keywords in both content (weight 1.0) and conditions (weight 2.0 in bm25(memory_search, 0, 1, 2))
    let id_strong = store
        .propose_lesson(
            "Resolve pg_bouncer connection saturation and query queuing by tuning pool_mode transaction and max_client_conn",
            "postgres pg_bouncer connection saturation under high concurrency",
            &ep,
        )
        .expect("failed to propose strong lesson");
    store
        .activate(&id_strong)
        .expect("failed to activate strong lesson");

    // Record 3: Unverified candidate (must not pollute active retrieval)
    let ep_unverified = store
        .record_episode("task-2", "failed verification", false)
        .expect("failed to record unverified episode");
    let _id_unverified = store
        .propose_lesson(
            "postgres pg_bouncer connection saturation unverified workaround",
            "postgres pg_bouncer connection saturation",
            &ep_unverified,
        )
        .expect("failed to propose unverified lesson");

    let retrieved = store
        .retrieve("task-retrieve", "postgres pg_bouncer connection saturation")
        .expect("failed to retrieve persona memories");

    assert_eq!(
        retrieved.memory_ids.len(),
        2,
        "Only active verified memories matching query tokens should be retrieved"
    );
    assert_eq!(
        retrieved.memory_ids[0], id_strong,
        "FTS5 BM25 + conditions weight must rank the multi-keyword condition+content match first"
    );
    assert_eq!(
        retrieved.memory_ids[1], id_weak,
        "Weak single-keyword match must rank second"
    );

    // Verify durable store direct construction & ranked retrieval works seamlessly alongside MemoryService
    let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
    rt.block_on(async {
        let durable = DurableMemoryStore::new(ws.path());
        let l1 = StructuredLesson {
            id: "d-1".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            task_title: "Tune pg_bouncer connection saturation".to_string(),
            trigger_error: Some("pg_bouncer max_client_conn reached".to_string()),
            remediation_applied: Some("Set pool_mode=transaction".to_string()),
            general_insight: "Transaction pooling prevents postgres connection saturation"
                .to_string(),
            success: true,
        };
        durable
            .save_structured_lesson(&l1)
            .await
            .expect("failed to save lesson");
        let top = durable
            .retrieve_ranked_lessons("pg_bouncer connection saturation", 1)
            .await
            .expect("failed to retrieve ranked lessons");
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0.id, "d-1");
        assert!(top[0].1 > 0.0);
    });
}
