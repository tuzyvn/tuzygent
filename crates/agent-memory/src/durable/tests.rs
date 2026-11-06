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
