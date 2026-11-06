#[cfg(test)]
mod unit_tests {
    use anyhow::Context;
    use crate::durable::fault::DurableMemoryFault;
    use crate::durable::policy::DurableMemoryFaultPolicy;
    use crate::durable::resilience::ResilientDurableMemory;
    use crate::durable::storage::StructuredLesson;
    use tokio::fs;

    #[tokio::test]
    async fn test_resilient_durable_memory_alias_and_builders() -> anyhow::Result<()> {
        let temp_dir =
            std::env::temp_dir().join(format!("test_rdm_{}", uuid::Uuid::new_v4().simple()));
        let mut mem: ResilientDurableMemory = ResilientDurableMemory::from_workspace(&temp_dir)
            .with_max_retries(5)
            .with_fault_policy(DurableMemoryFaultPolicy::intermittent(
                DurableMemoryFault::TransientIo,
                2,
            ));

        assert_eq!(mem.max_retries(), 5);
        assert_eq!(mem.saved_count(), 0);

        let res = mem.save_lesson("Architecture resilience verified").await;
        assert!(res.is_ok());
        assert_eq!(mem.saved_count(), 1);
        assert!(mem.recovered_count() > 0);

        let loaded = mem
            .load_relevant_context("resilience")
            .await
            .context("failed to load relevant context")?;
        assert!(loaded.contains("Architecture resilience verified"));

        let _ = fs::remove_dir_all(&temp_dir).await;
        Ok(())
    }

    #[tokio::test]
    async fn test_resilient_durable_memory_open_constructor() -> anyhow::Result<()> {
        let temp_dir =
            std::env::temp_dir().join(format!("test_rdm_open_{}", uuid::Uuid::new_v4().simple()));
        let mut mem = ResilientDurableMemory::open(&temp_dir);

        let lesson = StructuredLesson {
            id: "les-open-01".to_string(),
            timestamp: "2026-11-06T00:00:00Z".to_string(),
            task_title: "Open constructor check".to_string(),
            trigger_error: None,
            remediation_applied: None,
            general_insight: "Resilient memory constructor works seamlessly".to_string(),
            success: true,
        };

        assert!(mem.save_structured_lesson(&lesson).await.is_ok());
        let lessons = mem
            .load_structured_lessons()
            .await
            .context("failed to load structured lessons")?;
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].id, "les-open-01");

        let _ = fs::remove_dir_all(&temp_dir).await;
        Ok(())
    }

    #[tokio::test]
    async fn test_resilient_durable_memory_safe_error_handling() -> anyhow::Result<()> {
        let temp_dir =
            std::env::temp_dir().join(format!("test_rdm_err_{}", uuid::Uuid::new_v4().simple()));
        let mut mem: ResilientDurableMemory = ResilientDurableMemory::from_workspace(&temp_dir)
            .with_max_retries(0)
            .with_fault_policy(DurableMemoryFaultPolicy::always(
                DurableMemoryFault::TransientIo,
            ));

        let res_ctx = mem.load_relevant_context("resilience").await;
        assert!(res_ctx.is_err(), "Must return Err on failure without panicking");

        let res_lessons = mem.load_structured_lessons().await;
        assert!(res_lessons.is_err(), "Must return Err on failure without panicking");

        let _ = fs::remove_dir_all(&temp_dir).await;
        Ok(())
    }

    #[test]
    fn test_bm25_tf_idl_doc_length_and_hybrid_lesson_relevance() -> anyhow::Result<()> {
        use crate::durable::ranking::{
            compute_bm25_score, compute_bm25_term_score, compute_idf, score_lesson_hybrid,
            score_lesson_relevance, Bm25Params,
        };
        use chrono::{Duration, Utc};

        let params = Bm25Params::default();
        let idf_rare = compute_idf(1, 20);
        let idf_common = compute_idf(15, 20);
        assert!(idf_rare > idf_common, "Rare token must have higher IDF than common token");
        assert!(idf_common > 0.0, "IDF must remain strictly positive");

        // TF monotonicity and saturation
        let s_tf1 = compute_bm25_term_score(1.0, 15, 15.0, idf_rare, params);
        let s_tf3 = compute_bm25_term_score(3.0, 15, 15.0, idf_rare, params);
        let s_tf10 = compute_bm25_term_score(10.0, 15, 15.0, idf_rare, params);
        assert!(s_tf1 < s_tf3 && s_tf3 < s_tf10, "BM25 term score must increase with TF");

        // Document length normalization
        let s_short = compute_bm25_term_score(1.0, 6, 18.0, idf_rare, params);
        let s_long = compute_bm25_term_score(1.0, 48, 18.0, idf_rare, params);
        assert!(s_short > s_long, "Shorter document with same TF must score higher");

        let corpus = vec![
            "database connection pool exhausted under spike",
            "service timeout error during deployment",
            "service log rotation error on worker",
        ];
        let rare_doc_score = compute_bm25_score("exhausted service", corpus[0], &corpus);
        let common_doc_score = compute_bm25_score("exhausted service", corpus[1], &corpus);
        assert!(
            rare_doc_score > common_doc_score,
            "Matching rare high-IDF token 'exhausted' must outscore common token 'service'"
        );

        let now = Utc::now();
        let recent_verified = StructuredLesson {
            id: "l-recent".to_string(),
            timestamp: now.to_rfc3339(),
            task_title: "Fix postgres deadlock in transaction".to_string(),
            trigger_error: Some("deadlock detected on lock_table".to_string()),
            remediation_applied: Some("Retry serializable transaction with backoff".to_string()),
            general_insight: "Order row locks consistently to prevent postgres deadlock".to_string(),
            success: true,
        };
        let old_failed = StructuredLesson {
            id: "l-old-failed".to_string(),
            timestamp: (now - Duration::days(120)).to_rfc3339(),
            task_title: "Fix postgres deadlock in transaction".to_string(),
            trigger_error: Some("deadlock detected on lock_table".to_string()),
            remediation_applied: None,
            general_insight: "Order row locks consistently to prevent postgres deadlock".to_string(),
            success: false,
        };

        let s_recent = score_lesson_relevance("postgres deadlock transaction", &recent_verified, now);
        let s_old = score_lesson_relevance("postgres deadlock transaction", &old_failed, now);
        assert!(s_recent > s_old, "Recent verified lesson must outscore old failed lesson");

        let breakdown = score_lesson_hybrid(
            "postgres deadlock transaction",
            &recent_verified,
            &[recent_verified.clone(), old_failed],
            now,
        );
        assert!((breakdown.coverage_ratio - 1.0).abs() < f64::EPSILON);
        assert!(breakdown.confidence_weight > 1.0);
        assert!(breakdown.recency_weight > 0.95);
        assert!(breakdown.final_score > 0.0);

        Ok(())
    }

    #[test]
    fn test_rank_documents_bm25_idf_tf_saturation_and_doc_length_norm() -> anyhow::Result<()> {
        use crate::durable::ranking::rank_documents_bm25;

        let docs = vec![
            // doc 0: only common token 'service' in a verbose padded document
            "service log rotation check on worker alpha beta gamma delta epsilon zeta eta theta",
            // doc 1: rare token 'quorum_split' in a verbose document
            "service cluster encountered quorum_split during routine health check on node alpha beta gamma delta",
            // doc 2: rare token 'quorum_split' + common token 'service' in a concise document
            "service raft quorum_split recovery",
            // doc 3: repeated 'quorum_split' with TF saturation in a concise document
            "service quorum_split quorum_split raft recovery",
            // doc 4: completely unrelated document
            "frontend css button alignment issue",
        ];

        let ranked = rank_documents_bm25("service quorum_split", &docs);
        assert_eq!(
            ranked.len(),
            4,
            "Unrelated document must be excluded from BM25 ranking"
        );

        // Highest should be doc 3 (higher TF on rare token + concise length)
        assert_eq!(ranked[0].0, docs[3]);
        // Second should be doc 2 (concise doc with rare token + common token)
        assert_eq!(ranked[1].0, docs[2]);
        // Third should be doc 1 (verbose doc with rare token + common token)
        assert_eq!(ranked[2].0, docs[1]);
        // Lowest matched should be doc 0 (only common token 'service')
        assert_eq!(ranked[3].0, docs[0]);

        // Verify TF saturation: doubling TF from 1 to 2 increases score by less than 2x
        let ratio = ranked[0].1 / ranked[1].1;
        assert!(
            ratio > 1.0 && ratio < 1.8,
            "BM25 TF saturation must sublinearly bound score growth (ratio = {})",
            ratio
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_durable_memory_store_load_relevant_context_hybrid_ranking() -> anyhow::Result<()> {
        use crate::durable::storage::DurableMemoryStore;

        let temp_dir = std::env::temp_dir().join(format!(
            "test_dms_hybrid_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = DurableMemoryStore::new(&temp_dir);

        // Save 3 lessons:
        // 1. Partial match with FAILURE tag and verbose padding
        let partial_failed = StructuredLesson {
            id: "les-partial-fail".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            task_title: "Investigate redis timeout in background worker".to_string(),
            trigger_error: Some("connection reset on worker pool alpha beta gamma delta".to_string()),
            remediation_applied: None,
            general_insight: "Worker retry loop failed to recover redis connection under load".to_string(),
            success: false,
        };
        store.save_structured_lesson(&partial_failed).await?;

        // 2. High-relevance SUCCESS lesson matching rare token 'lua_script' + 'redis' + 'atomic'
        let exact_verified = StructuredLesson {
            id: "les-exact-success".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            task_title: "Fix redis lua_script atomic lock contention".to_string(),
            trigger_error: Some("ERR redis lua_script execution conflict".to_string()),
            remediation_applied: Some("Use atomic EVALSHA with single key slot".to_string()),
            general_insight: "Ensure redis lua_script operations remain atomic".to_string(),
            success: true,
        };
        store.save_structured_lesson(&exact_verified).await?;

        // 3. Unrelated lesson that should appear only in Recent Lessons Learned
        store
            .save_lesson("Unrelated markdown formatting cleanup for documentation")
            .await?;

        let ctx = store
            .load_relevant_context("Fix redis lua_script atomic lock")
            .await
            .context("failed to load relevant context")?;

        assert!(ctx.contains("## High Relevance to Current Goal:"));
        assert!(ctx.contains("## Recent Lessons Learned:"));

        let pos_exact = ctx
            .find("Fix redis lua_script atomic lock contention")
            .context("exact verified lesson missing from context")?;
        let pos_partial = ctx
            .find("Investigate redis timeout in background worker")
            .context("partial failed lesson missing from context")?;
        let pos_recent_header = ctx
            .find("## Recent Lessons Learned:")
            .context("recent lessons header missing")?;
        let pos_unrelated = ctx
            .find("Unrelated markdown formatting cleanup")
            .context("unrelated lesson missing from recent section")?;

        assert!(
            pos_exact < pos_partial,
            "High-IDF full-coverage [SUCCESS] lesson must rank above partial [FAILURE] lesson"
        );
        assert!(
            pos_partial < pos_recent_header && pos_recent_header < pos_unrelated,
            "Matched lessons must appear in High Relevance section before Recent Lessons section"
        );

        let _ = fs::remove_dir_all(&temp_dir).await;
        Ok(())
    }

    #[tokio::test]
    async fn test_retrieve_ranked_lessons_hybrid_semantic_scoring_and_top_k() -> anyhow::Result<()> {
        use crate::durable::ranking::{rank_lesson_blocks, score_lesson_hybrid};
        use crate::durable::storage::DurableMemoryStore;
        use chrono::{Duration, Utc};

        let temp_dir = std::env::temp_dir().join(format!(
            "test_dms_retrieve_ranked_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = DurableMemoryStore::new(&temp_dir);
        let now = Utc::now();

        let l_noise = StructuredLesson {
            id: "les-unrelated".to_string(),
            timestamp: now.to_rfc3339(),
            task_title: "Update README badge links".to_string(),
            trigger_error: None,
            remediation_applied: None,
            general_insight: "Ensure CI status badge points to main branch".to_string(),
            success: true,
        };
        let l_common_only = StructuredLesson {
            id: "les-common-only".to_string(),
            timestamp: now.to_rfc3339(),
            task_title: "Check database connection timeout in staging".to_string(),
            trigger_error: Some("connection timeout during healthcheck".to_string()),
            remediation_applied: Some("Increase connection timeout to 10s".to_string()),
            general_insight: "Staging database needs higher timeout threshold".to_string(),
            success: true,
        };
        let l_stale_multi = StructuredLesson {
            id: "les-stale-multi".to_string(),
            timestamp: (now - Duration::days(120)).to_rfc3339(),
            task_title: "Debug mvcc_snapshot_conflict timeout in database".to_string(),
            trigger_error: Some("mvcc_snapshot_conflict serialization failure on timeout".to_string()),
            remediation_applied: None,
            general_insight: "Concurrent writers triggered mvcc_snapshot_conflict".to_string(),
            success: false,
        };
        let l_best_hybrid = StructuredLesson {
            id: "les-best-hybrid".to_string(),
            timestamp: now.to_rfc3339(),
            task_title: "Resolve mvcc_snapshot_conflict serialization timeout in database".to_string(),
            trigger_error: Some("mvcc_snapshot_conflict serialization error under load".to_string()),
            remediation_applied: Some("Retry transaction with jittered exponential backoff".to_string()),
            general_insight: "Jittered retry resolves mvcc_snapshot_conflict serialization timeout".to_string(),
            success: true,
        };

        for lesson in [&l_noise, &l_common_only, &l_stale_multi, &l_best_hybrid] {
            store.save_structured_lesson(lesson).await?;
        }

        let query = "mvcc_snapshot_conflict serialization timeout database";
        let corpus = vec![
            l_noise.clone(),
            l_common_only.clone(),
            l_stale_multi.clone(),
            l_best_hybrid.clone(),
        ];

        let b_best = score_lesson_hybrid(query, &l_best_hybrid, &corpus, now);
        let b_stale = score_lesson_hybrid(query, &l_stale_multi, &corpus, now);
        let b_common = score_lesson_hybrid(query, &l_common_only, &corpus, now);
        assert!(b_best.bm25_score > b_common.bm25_score);
        assert!(b_best.coverage_ratio > b_common.coverage_ratio);
        assert!(b_best.confidence_weight > b_stale.confidence_weight);
        assert!(b_best.recency_weight > b_stale.recency_weight);
        assert!(b_best.final_score > b_stale.final_score && b_stale.final_score > b_common.final_score);

        let top2 = store.retrieve_ranked_lessons(query, 2).await?;
        assert_eq!(top2.len(), 2, "top_k=2 must truncate results to 2 items");
        assert_eq!(top2[0].0.id, "les-best-hybrid");
        assert_eq!(top2[1].0.id, "les-stale-multi");

        let blocks = vec![
            format!(
                "- [Learned {}] [FAILURE] Goal 'Old': mvcc_snapshot_conflict timeout",
                (now - Duration::days(90)).to_rfc3339()
            ),
            format!(
                "- [Learned {}] [SUCCESS] Goal 'Recent': mvcc_snapshot_conflict serialization timeout solved",
                now.to_rfc3339()
            ),
        ];
        let ranked_blocks = rank_lesson_blocks(query, &blocks, now);
        assert_eq!(ranked_blocks.len(), 2);
        assert!(ranked_blocks[0].0.contains("Recent"));

        let _ = fs::remove_dir_all(&temp_dir).await;
        Ok(())
    }

    #[test]
    fn test_topic_4b1ff5_durable_tests_contain_zero_unwrap() {
        let pattern = format!(".{}()", "unwrap");
        let file_content = include_str!("tests.rs");
        let unwrap_calls: Vec<(usize, &str)> = file_content
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                !trimmed.starts_with("//") && trimmed.contains(&pattern)
            })
            .collect();

        assert!(
            unwrap_calls.is_empty(),
            "crates/agent-memory/src/durable/tests.rs must contain zero unwrap calls, found: {:?}",
            unwrap_calls
        );
    }
}
