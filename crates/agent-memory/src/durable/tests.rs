#[cfg(test)]
mod unit_tests {
    use crate::durable::fault::DurableMemoryFault;
    use crate::durable::policy::DurableMemoryFaultPolicy;
    use crate::durable::resilience::ResilientDurableMemory;
    use crate::durable::storage::StructuredLesson;
    use tokio::fs;

    #[tokio::test]
    async fn test_resilient_durable_memory_alias_and_builders() {
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

        let loaded = mem.load_relevant_context("resilience").await.unwrap();
        assert!(loaded.contains("Architecture resilience verified"));

        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_resilient_durable_memory_open_constructor() {
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
        let lessons = mem.load_structured_lessons().await.unwrap();
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].id, "les-open-01");

        let _ = fs::remove_dir_all(&temp_dir).await;
    }
}
