use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
};
use agent_memory::{DurableMemoryStore, PersonaLoader, ScratchpadManager, SkillRegistry};
use std::path::PathBuf;

use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(1);

#[tokio::test]
async fn corrupt_structured_lessons_are_not_overwritten() {
    let dir = create_temp_dir("corrupt_structured");
    let path = dir.join("storage/harness_memory/structured_lessons.json");
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, b"invalid json").await.unwrap();
    let lesson = agent_memory::durable::StructuredLesson {
        id: "1".into(),
        timestamp: "now".into(),
        task_title: "test".into(),
        trigger_error: None,
        remediation_applied: None,
        general_insight: "test".into(),
        success: true,
    };
    assert!(DurableMemoryStore::new(&dir)
        .save_structured_lesson(&lesson)
        .await
        .is_err());
    assert_eq!(tokio::fs::read(&path).await.unwrap(), b"invalid json");
    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn concurrent_structured_lesson_writers_preserve_updates() {
    let dir = create_temp_dir("concurrent_structured");
    let mut tasks = Vec::new();
    for i in 0..32 {
        let store = DurableMemoryStore::new(&dir);
        tasks.push(tokio::spawn(async move {
            let lesson = agent_memory::StructuredLesson {
                id: i.to_string(),
                timestamp: "now".into(),
                task_title: "test".into(),
                trigger_error: None,
                remediation_applied: None,
                general_insight: "test".into(),
                success: true,
            };
            store.save_structured_lesson(&lesson).await.unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        DurableMemoryStore::new(&dir)
            .load_structured_lessons()
            .await
            .unwrap()
            .len(),
        32
    );
    let _ = tokio::fs::remove_dir_all(dir).await;
}

fn create_temp_dir(prefix: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("agent_mem_test_{}_{}_{}", prefix, nanos, id));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn test_durable_memory_short_technical_keywords() {
    let temp_dir = create_temp_dir("keywords");
    let store = DurableMemoryStore::new(&temp_dir);

    // Save lessons with short technical terms (2-3 chars)
    store
        .save_lesson("Fixed bug in REST API endpoint using git rebase")
        .await
        .unwrap();
    store
        .save_lesson("Deploy PostgreSQL DB with Docker Compose")
        .await
        .unwrap();

    // Query with 2-3 char technical words that were previously dropped by w.len() > 3
    let api_ctx = store.load_relevant_context("fix api bug").await.unwrap();
    assert!(
        api_ctx.contains("Fixed bug in REST API endpoint"),
        "Context should match short keywords 'fix', 'api', 'bug'"
    );

    let git_ctx = store.load_relevant_context("git commands").await.unwrap();
    assert!(
        git_ctx.contains("REST API endpoint using git rebase"),
        "Context should match 'git'"
    );

    let db_ctx = store.load_relevant_context("db migrations").await.unwrap();
    assert!(
        db_ctx.contains("PostgreSQL DB with Docker"),
        "Context should match 'db'"
    );

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_durable_memory_multiline_lesson_block() {
    let temp_dir = create_temp_dir("multiline");
    let store = DurableMemoryStore::new(&temp_dir);

    let multiline_lesson =
        "Optimization insight:\nLine 2: Use tokio::sync::mpsc\nLine 3: Benchmark shows 4x speedup";
    store.save_lesson(multiline_lesson).await.unwrap();

    let ctx = store
        .load_relevant_context("tokio optimization")
        .await
        .unwrap();
    assert!(ctx.contains("Optimization insight:"));
    assert!(
        ctx.contains("Line 2: Use tokio::sync::mpsc"),
        "Multi-line lesson line 2 should not be lost"
    );
    assert!(
        ctx.contains("Line 3: Benchmark shows 4x speedup"),
        "Multi-line lesson line 3 should not be lost"
    );

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_scratchpad_append_and_recent_entries() {
    let temp_dir = create_temp_dir("scratchpad");
    let scratchpad = ScratchpadManager::new(&temp_dir);

    scratchpad.append_log("Step 1: Planning").await.unwrap();
    scratchpad.append_log("Step 2: Coding").await.unwrap();
    scratchpad.append_log("Step 3: Testing").await.unwrap();
    scratchpad.append_log("Step 4: Done").await.unwrap();

    let recent = scratchpad.read_recent_entries(2).await.unwrap();
    assert_eq!(recent.len(), 2);
    assert!(recent[0].contains("Step 3: Testing"));
    assert!(recent[1].contains("Step 4: Done"));

    scratchpad.clear().await.unwrap();
    let after_clear = scratchpad.read().await.unwrap();
    assert!(after_clear.is_empty());

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_skill_registry_security_and_frontmatter() {
    let temp_dir = create_temp_dir("skills");
    let registry = SkillRegistry::new(&temp_dir);

    // 1. Path traversal security checks
    assert!(registry.load_skill("../../etc/passwd").await.is_err());
    assert!(registry.load_skill("../sensitive").await.is_err());
    assert!(registry.load_skill("").await.is_err());
    assert!(registry.load_skill("sub/dir").await.is_err());

    // 2. Create a valid skill
    let skill_dir = temp_dir.join("code-review");
    tokio::fs::create_dir_all(&skill_dir).await.unwrap();
    let skill_content = r#"---
name: Code Review Expert
description: Automated static analysis and code smell reviewer
---

# Code Review Instructions
Inspect code for safety and style.
"#;
    tokio::fs::write(skill_dir.join("SKILL.md"), skill_content)
        .await
        .unwrap();

    // 3. List skills
    let skills = registry.list_available_skills().await.unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "Code Review Expert");
    assert_eq!(skills[0].id, "code-review");
    assert!(skills[0].description.contains("Automated static analysis"));

    // 4. Load by directory id or declared name
    let loaded_by_id = registry.load_skill("code-review").await.unwrap();
    assert!(loaded_by_id.contains("Code Review Instructions"));

    let loaded_by_name = registry.load_skill("Code Review Expert").await.unwrap();
    assert!(loaded_by_name.contains("Code Review Instructions"));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_persona_markdown_roundtrip() {
    let temp_dir = create_temp_dir("persona");
    let persona_dir = temp_dir.join("secops-agent");

    let original = AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::from_str("secops-007"),
            name: "Agent James".to_string(),
            description: "Elite security operations auditor".to_string(),
            autonomy_level: AutonomyLevel::Supervised,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: "SECOPS".to_string(),
            title: "Senior Security Specialist".to_string(),
            primary_domain: "Cybersecurity & Pentesting".to_string(),
            capabilities: vec!["audit".to_string(), "red-team".to_string()],
        },
        soul: AgentSoul {
            persona: "Analytical, cautious, vigilant".to_string(),
            tone_of_voice: "Firm and professional".to_string(),
            core_values: vec!["Integrity".to_string(), "Zero Trust".to_string()],
            voice_profile: None,
        },
        mission: AgentMission {
            high_level_objective: "Audit zero day vulnerabilities".to_string(),
            kpis: vec![
                "100% test pass rate".to_string(),
                "Zero red line violation".to_string(),
            ],
            constraints: vec![
                "Never exfiltrate keys".to_string(),
                "Require approval for port scans".to_string(),
            ],
        },
    };

    // Export to dir
    PersonaLoader::export_to_dir(&original, &persona_dir)
        .await
        .unwrap();

    // Delete profile.json to force loading exclusively from IDENTITY.md, SOUL.md, MISSION.md
    tokio::fs::remove_file(persona_dir.join("profile.json"))
        .await
        .unwrap();

    // Load from markdown
    let loaded = PersonaLoader::load_from_markdown_files(&persona_dir)
        .await
        .unwrap();

    assert_eq!(loaded.identity.name, "Agent James");
    assert_eq!(loaded.identity.id.0, "secops-007");
    assert_eq!(loaded.identity.autonomy_level, AutonomyLevel::Supervised);
    assert_eq!(loaded.role.title, "Senior Security Specialist");
    assert_eq!(loaded.role.primary_domain, "Cybersecurity & Pentesting");
    assert!(loaded
        .soul
        .persona
        .contains("Analytical, cautious, vigilant"));
    assert_eq!(loaded.soul.tone_of_voice, "Firm and professional");
    assert_eq!(loaded.soul.core_values, vec!["Integrity", "Zero Trust"]);
    assert!(loaded
        .mission
        .high_level_objective
        .contains("Audit zero day vulnerabilities"));
    assert_eq!(
        loaded.mission.constraints,
        vec!["Never exfiltrate keys", "Require approval for port scans"]
    );

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}
