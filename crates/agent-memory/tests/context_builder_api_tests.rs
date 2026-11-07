use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
};
use agent_memory::{ContextBuilder, ContextBuilderError, SkillMetadata};
use std::path::PathBuf;

fn profile() -> AgentPersonaProfile {
    AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::from_str("builder-test-001"),
            name: "Builder Test Agent".to_string(),
            description: "A test persona".to_string(),
            autonomy_level: AutonomyLevel::Supervised,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: "TESTER".to_string(),
            title: "Context Builder Tester".to_string(),
            primary_domain: "Testing".to_string(),
            capabilities: vec!["context assembly".to_string()],
        },
        soul: AgentSoul {
            persona: "Careful and clear".to_string(),
            tone_of_voice: "Direct".to_string(),
            core_values: vec!["Accuracy".to_string()],
            voice_profile: None,
        },
        mission: AgentMission {
            high_level_objective: "Verify context construction".to_string(),
            kpis: vec![],
            constraints: vec![],
        },
    }
}

fn skill(description: &str) -> SkillMetadata {
    SkillMetadata {
        id: "test-skill".to_string(),
        name: "Test Skill".to_string(),
        description: description.to_string(),
        path: PathBuf::from("skills/test.md"),
        content: "instructions".to_string(),
        frontmatter: None,
    }
}

#[test]
fn accessors_report_defaults_and_all_configured_values() {
    let empty = ContextBuilder::new("goal");
    assert_eq!(empty.goal(), "goal");
    assert!(empty.persona().is_none());
    assert!(empty.skills().is_empty());
    assert!(empty.lessons().is_empty());
    assert!(empty.recent_logs().is_empty());
    assert!(empty.subtask_history().is_empty());
    assert!(empty.artifacts().is_empty());
    assert_eq!(empty.max_total_chars(), 16_000);

    let persona = profile();
    let skills = vec![skill("skill description")];
    let lessons = vec!["lesson one".to_string()];
    let logs = vec!["recent event".to_string()];
    let history = vec![("step-1".to_string(), "step result".to_string())];
    let artifacts = vec!["report.md".to_string()];
    let configured = ContextBuilder::new("configured goal")
        .with_persona(Some(persona.clone()))
        .with_skills(skills)
        .with_lessons(lessons.clone())
        .with_recent_logs(logs.clone())
        .with_subtask_history(history.clone())
        .with_artifacts(artifacts.clone())
        .with_max_total_chars(321);

    assert_eq!(configured.goal(), "configured goal");
    assert_eq!(configured.persona(), Some(&persona));
    assert_eq!(configured.skills()[0].id, "test-skill");
    assert_eq!(configured.lessons(), lessons);
    assert_eq!(configured.recent_logs(), logs);
    assert_eq!(configured.subtask_history(), history);
    assert_eq!(configured.artifacts(), artifacts);
    assert_eq!(configured.max_total_chars(), 321);
}

#[test]
fn persona_skill_and_history_setters_replace_values_and_affect_build_output() {
    let description: String = "é".repeat(201);
    let original = ContextBuilder::new("goal")
        .with_persona(Some(profile()))
        .with_skills(vec![skill(&description)])
        .with_subtask_history(vec![("old".to_string(), "old output".to_string())]);

    let (system, working) = original.clone().build();
    assert!(system.contains("### AGENT IDENTITY & VOICE:"));
    assert!(system.contains("Builder Test Agent"));
    assert!(system.contains("### RELEVANT OPERATIONAL SKILLS:"));
    assert!(system.contains(&format!("- [test-skill] Test Skill: {}", "é".repeat(200))));
    assert!(!system.contains(&"é".repeat(201)));
    assert!(working.contains("Step old"));

    let replaced = original
        .with_persona(None)
        .with_skills(Vec::new())
        .with_subtask_history(vec![("new".to_string(), "new output".to_string())]);
    assert!(replaced.persona().is_none());
    assert!(replaced.skills().is_empty());
    assert_eq!(
        replaced.subtask_history(),
        &[("new".to_string(), "new output".to_string())]
    );

    let (system, working) = replaced.build();
    assert!(!system.contains("AGENT IDENTITY & VOICE"));
    assert!(!system.contains("RELEVANT OPERATIONAL SKILLS"));
    assert!(!working.contains("Step old"));
    assert!(working.contains("Step new"));
}

#[test]
fn validate_rejects_empty_or_whitespace_goals_and_accepts_nonempty_goals() {
    assert_eq!(
        ContextBuilder::new("").validate(),
        Err(ContextBuilderError::EmptyGoal)
    );
    assert_eq!(
        ContextBuilder::new(" \n\t ").validate(),
        Err(ContextBuilderError::EmptyGoal)
    );
    assert_eq!(ContextBuilder::new(" x ").validate(), Ok(()));
}

#[test]
fn build_safe_returns_built_context_and_propagates_validation_error() {
    let builder = ContextBuilder::new("safe goal")
        .with_lessons(vec!["guardrail".to_string()])
        .with_recent_logs(vec!["event".to_string()]);
    let safely_built = builder.build_safe().expect("valid goal should build");
    assert_eq!(safely_built, builder.clone().build());
    assert!(safely_built.0.contains("safe goal"));
    assert!(safely_built.0.contains("guardrail"));
    assert!(safely_built.1.contains("event"));

    assert_eq!(
        ContextBuilder::new(" \t ").build_safe(),
        Err(ContextBuilderError::EmptyGoal)
    );
}

#[test]
fn zero_character_budget_is_preserved_and_truncates_working_context() {
    let builder = ContextBuilder::new("goal")
        .with_recent_logs(vec!["event".to_string()])
        .with_max_total_chars(0);
    assert_eq!(builder.max_total_chars(), 0);

    let (system, working) = builder
        .build_safe()
        .expect("goal remains valid at zero budget");
    assert!(system.contains("goal"));
    assert!(working.is_empty());
}
