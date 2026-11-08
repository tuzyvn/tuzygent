    use super::AutonomousOodaLoop;
    use agent_domain::{
        AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul,
        AutonomyLevel, GuardrailPolicy,
    };
    use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
    use agent_memory::{DurableMemoryStore, ScratchpadManager};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct UnusedEngine;

    #[async_trait]
    impl ComputeEngine for UnusedEngine {
        async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
            Err(EngineError::ProcessFailed(
                "engine execution is not expected in public API tests".into(),
            ))
        }

        fn engine_kind(&self) -> EngineKind {
            EngineKind::Mock
        }
    }

    fn test_loop() -> AutonomousOodaLoop {
        let workspace = std::env::temp_dir().join(format!(
            "agent_runtime_ooda_public_api_{}",
            uuid::Uuid::new_v4()
        ));
        let workspace_dir = workspace.to_string_lossy();
        AutonomousOodaLoop::new(
            Arc::new(UnusedEngine),
            GuardrailPolicy::default_enterprise(),
            ScratchpadManager::new(&workspace),
            DurableMemoryStore::new(&workspace),
            &workspace_dir,
        )
    }

    #[tokio::test]
    async fn unused_engine_returns_an_error_instead_of_panicking() {
        let ooda = test_loop();

        let result = ooda.engine.execute(&EngineInput::default()).await;

        assert!(matches!(
            result,
            Err(EngineError::ProcessFailed(message))
                if message == "engine execution is not expected in public API tests"
        ));
    }

    fn persona(id: &str) -> AgentPersonaProfile {
        AgentPersonaProfile {
            identity: AgentIdentity {
                id: AgentId::from_str(id),
                name: "Test persona".into(),
                description: String::new(),
                autonomy_level: AutonomyLevel::Supervised,
                created_at: chrono::Utc::now(),
            },
            role: AgentRole {
                role_code: "TEST".into(),
                title: "Tester".into(),
                primary_domain: "Testing".into(),
                capabilities: vec![],
            },
            soul: AgentSoul {
                persona: "Careful".into(),
                tone_of_voice: "Direct".into(),
                core_values: vec![],
                voice_profile: None,
            },
            mission: AgentMission {
                high_level_objective: "Test API behavior".into(),
                kpis: vec![],
                constraints: vec![],
            },
        }
    }

    #[test]
    fn with_persona_stores_the_last_profile_and_accepts_an_empty_profile() {
        let first = persona("first");
        let replacement = persona("");

        let configured = test_loop()
            .with_persona(first)
            .with_persona(replacement.clone());

        assert_eq!(configured.persona, Some(replacement));
    }

    #[test]
    fn with_timeout_seconds_clamps_zero_to_one_second() {
        let configured = test_loop().with_timeout_seconds(0);

        assert_eq!(configured.timeout_seconds, 1);
    }

    #[tokio::test]
    async fn supplement_controller_clones_share_the_mailbox() -> Result<(), String> {
        let ooda = test_loop();
        let controller = ooda.supplement_controller();
        let another_clone = ooda.supplement_controller();

        assert!(another_clone.try_recv().await.is_none());
        controller
            .inject(crate::supplement::InFlightSupplement::new(
                "reviewer",
                "Keep the original objective",
            ))
            .await?;

        let received = another_clone
            .try_recv()
            .await
            .ok_or_else(|| "expected the injected supplement to be available".to_owned())?;
        assert_eq!(received.author_name, "reviewer");
        assert_eq!(received.content, "Keep the original objective");
        assert!(controller.try_recv().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn try_recv_reports_missing_supplement_without_panicking() {
        let ooda = test_loop();

        let missing = ooda.supplement_controller().try_recv().await;

        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn isolated_scratchpad_does_not_expose_or_pollute_parent_memory() {
        let ooda = test_loop();
        ooda.scratchpad
            .append_log("private parent context")
            .await
            .unwrap();

        let isolated = ooda.isolated_scratchpad("untrusted-subagent");

        assert_eq!(isolated.access(), agent_memory::ScopeAccess::Isolated);
        assert_eq!(
            isolated.read_parent().await.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        isolated.append_log("local child context").await.unwrap();
        assert!(isolated.read().await.unwrap().contains("local child context"));
        assert!(!ooda
            .scratchpad
            .read()
            .await
            .unwrap()
            .contains("local child context"));
    }

    #[tokio::test]
    async fn scoped_scratchpad_can_read_parent_and_keeps_child_writes_local() {
        let ooda = test_loop();
        ooda.scratchpad.append_log("shared parent context").await.unwrap();

        let scoped = ooda.scoped_scratchpad("reviewer", agent_memory::ScopeAccess::ReadWrite);

        assert_eq!(scoped.access(), agent_memory::ScopeAccess::ReadWrite);
        assert_eq!(scoped.parent_scope_id(), Some("root"));
        assert!(scoped
            .read_parent()
            .await
            .unwrap()
            .contains("shared parent context"));
        scoped.append_log("reviewer findings").await.unwrap();
        assert!(scoped.read().await.unwrap().contains("reviewer findings"));
        assert!(!ooda
            .scratchpad
            .read()
            .await
            .unwrap()
            .contains("reviewer findings"));
    }
