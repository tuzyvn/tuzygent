use agent_memory::{SkillRegistry, SkillValidationError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("skills_hot_reload_{}_{}_{}", prefix, nanos, id));
        std::fs::create_dir_all(&dir).expect("Failed to create temporary directory for test");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn test_dynamic_skill_detection_and_hot_reload() {
    let temp_guard = TempDirGuard::new("detection");
    let skills_dir = temp_guard.path();

    let mut registry = SkillRegistry::new(skills_dir);

    // Initial state: Registry has not loaded any skills yet
    assert_eq!(registry.cached_skills().len(), 0);
    let initial_skills = registry
        .reload_skills()
        .await
        .expect("reload_skills failed");
    assert_eq!(initial_skills.len(), 0);
    assert_eq!(registry.cached_skills().len(), 0);

    // 1. Dynamically add the first skill (folder structure: <dir>/SKILL.md)
    let alpha_dir = skills_dir.join("alpha-skill");
    tokio::fs::create_dir_all(&alpha_dir)
        .await
        .expect("Failed to create alpha_dir");
    let alpha_content = r#"---
name: Alpha Skill
description: Core alpha intelligence skill
version: 1.0.0
author: Antigravity Team
tags:
  - intelligence
  - analysis
---

# Alpha Skill Instructions
Execute alpha logic with precision.
"#;
    tokio::fs::write(alpha_dir.join("SKILL.md"), alpha_content)
        .await
        .expect("Failed to write alpha SKILL.md");

    // Hot-reload registry dynamically at runtime
    let reloaded_1 = registry
        .reload_skills()
        .await
        .expect("Failed to reload skills");
    assert_eq!(reloaded_1.len(), 1);
    assert_eq!(registry.cached_skills().len(), 1);

    let cached_alpha = registry
        .get_cached_skill("alpha-skill")
        .expect("alpha-skill should be found by folder ID");
    assert_eq!(cached_alpha.name, "Alpha Skill");
    assert_eq!(cached_alpha.description, "Core alpha intelligence skill");
    assert!(cached_alpha
        .content
        .contains("Execute alpha logic with precision"));

    // Verify lookup by declared name in frontmatter
    let cached_by_name = registry
        .get_cached_skill("Alpha Skill")
        .expect("alpha-skill should be found by declared name");
    assert_eq!(cached_by_name.id, "alpha-skill");

    // Verify frontmatter structured metadata
    if let Some(fm) = &cached_alpha.frontmatter {
        assert_eq!(fm.name, "Alpha Skill");
        assert_eq!(fm.version.as_deref(), Some("1.0.0"));
        assert_eq!(fm.author.as_deref(), Some("Antigravity Team"));
        assert_eq!(
            fm.tags,
            vec!["intelligence".to_string(), "analysis".to_string()]
        );
    } else {
        panic!("Expected frontmatter to be parsed and present on SkillMetadata");
    }

    // 2. Dynamically add a second skill (standalone markdown file: <stem>.md)
    let beta_content = r#"---
name: Beta Database Optimizer
description: Query planner optimization skill
version: 2.1.0
---

# Beta Instructions
Optimize queries and analyze indexes.
"#;
    tokio::fs::write(skills_dir.join("beta-optimizer.md"), beta_content)
        .await
        .expect("Failed to write beta-optimizer.md");

    // Dynamic hot reload again
    let reloaded_2 = registry
        .reload_skills()
        .await
        .expect("Failed to reload skills");
    assert_eq!(reloaded_2.len(), 2);
    assert_eq!(registry.cached_skills().len(), 2);

    let cached_beta = registry
        .get_cached_skill("beta-optimizer")
        .expect("beta-optimizer should be found in cache");
    assert_eq!(cached_beta.name, "Beta Database Optimizer");

    // Verify load_skill retrieves cached content
    let loaded_content = registry
        .load_skill("Beta Database Optimizer")
        .await
        .expect("load_skill by name should succeed");
    assert!(loaded_content.contains("Optimize queries and analyze indexes"));
}

#[tokio::test]
async fn test_dynamic_skill_update_and_hot_reload() {
    let temp_guard = TempDirGuard::new("update");
    let skills_dir = temp_guard.path();

    let mut registry = SkillRegistry::new(skills_dir);

    let skill_dir = skills_dir.join("live-skill");
    tokio::fs::create_dir_all(&skill_dir)
        .await
        .expect("Failed to create skill_dir");
    let initial_content = r#"---
name: Live Config
description: Version 1 Initial Spec
---
Initial body content v1.
"#;
    tokio::fs::write(skill_dir.join("SKILL.md"), initial_content)
        .await
        .expect("Failed to write initial SKILL.md");

    registry
        .reload_skills()
        .await
        .expect("Initial reload failed");
    assert_eq!(
        registry.get_cached_skill("live-skill").unwrap().description,
        "Version 1 Initial Spec"
    );
    assert!(registry
        .load_skill("live-skill")
        .await
        .unwrap()
        .contains("v1"));

    // Modify skill file on disk at runtime
    let updated_content = r#"---
name: Live Config
description: Version 2 Updated Spec
---
Updated body content v2 with enhanced instructions.
"#;
    tokio::fs::write(skill_dir.join("SKILL.md"), updated_content)
        .await
        .expect("Failed to write updated SKILL.md");

    // Trigger dynamic hot-reload
    registry
        .reload_skills()
        .await
        .expect("Update reload failed");
    assert_eq!(
        registry.get_cached_skill("live-skill").unwrap().description,
        "Version 2 Updated Spec"
    );
    assert!(registry
        .load_skill("live-skill")
        .await
        .unwrap()
        .contains("v2 with enhanced instructions"));
}

#[tokio::test]
async fn test_dynamic_skill_deletion_and_hot_reload() {
    let temp_guard = TempDirGuard::new("deletion");
    let skills_dir = temp_guard.path();

    let mut registry = SkillRegistry::new(skills_dir);

    // Create 2 skills
    let skill_a_dir = skills_dir.join("skill-a");
    let skill_b_dir = skills_dir.join("skill-b");
    tokio::fs::create_dir_all(&skill_a_dir).await.unwrap();
    tokio::fs::create_dir_all(&skill_b_dir).await.unwrap();

    tokio::fs::write(
        skill_a_dir.join("SKILL.md"),
        "---\nname: Skill A\ndescription: Skill A Description\n---\n# Body A",
    )
    .await
    .unwrap();
    tokio::fs::write(
        skill_b_dir.join("SKILL.md"),
        "---\nname: Skill B\ndescription: Skill B Description\n---\n# Body B",
    )
    .await
    .unwrap();

    registry.reload_skills().await.unwrap();
    assert_eq!(registry.cached_skills().len(), 2);

    // Delete skill-b from disk at runtime
    tokio::fs::remove_dir_all(&skill_b_dir).await.unwrap();

    // Hot-reload
    let reloaded = registry.reload_skills().await.unwrap();
    assert_eq!(reloaded.len(), 1);
    assert_eq!(registry.cached_skills().len(), 1);
    assert!(registry.get_cached_skill("skill-a").is_some());
    assert!(registry.get_cached_skill("skill-b").is_none());

    // load_skill for deleted skill should return NotFound error
    assert!(registry.load_skill("skill-b").await.is_err());
}

#[test]
fn test_frontmatter_schema_validation() {
    // 1. Valid frontmatter with full fields
    let valid_full = r#"---
name: Full Schema Skill
description: Detailed schema validation test
version: 3.0.0
author: Quality Lead
tags:
  - test
  - validation
---
# Body
"#;
    let validated =
        SkillRegistry::validate_frontmatter(valid_full).expect("Schema should validate");
    assert_eq!(validated.name, "Full Schema Skill");
    assert_eq!(validated.description, "Detailed schema validation test");
    assert_eq!(validated.version.as_deref(), Some("3.0.0"));
    assert_eq!(validated.author.as_deref(), Some("Quality Lead"));
    assert_eq!(
        validated.tags,
        vec!["test".to_string(), "validation".to_string()]
    );

    // 2. Missing frontmatter delimiters
    let no_frontmatter = "# Just Markdown\nNo YAML frontmatter at all.";
    assert_eq!(
        SkillRegistry::validate_frontmatter(no_frontmatter).unwrap_err(),
        SkillValidationError::MissingFrontmatter
    );

    // 3. Invalid YAML syntax
    let bad_yaml = "---\n: : bad syntax : [\n---\n# Body";
    match SkillRegistry::validate_frontmatter(bad_yaml) {
        Err(SkillValidationError::InvalidYaml(_)) => {}
        other => panic!("Expected InvalidYaml error, got {:?}", other),
    }

    // 4. Missing required 'name' field
    let missing_name = "---\ndescription: Skill without name\n---\n";
    assert_eq!(
        SkillRegistry::validate_frontmatter(missing_name).unwrap_err(),
        SkillValidationError::MissingName
    );

    // 5. Empty or whitespace 'name'
    let empty_name = "---\nname: '   '\ndescription: Skill with blank name\n---\n";
    assert_eq!(
        SkillRegistry::validate_frontmatter(empty_name).unwrap_err(),
        SkillValidationError::MissingName
    );

    // 6. Missing required 'description' field
    let missing_desc = "---\nname: Only Name\n---\n";
    assert_eq!(
        SkillRegistry::validate_frontmatter(missing_desc).unwrap_err(),
        SkillValidationError::MissingDescription
    );

    // 7. Empty or whitespace 'description'
    let empty_desc = "---\nname: Valid Name\ndescription: ''\n---\n";
    assert_eq!(
        SkillRegistry::validate_frontmatter(empty_desc).unwrap_err(),
        SkillValidationError::MissingDescription
    );
}

#[tokio::test]
async fn test_hot_reload_skips_invalid_skills_and_recovers() {
    let temp_guard = TempDirGuard::new("invalid_filter");
    let skills_dir = temp_guard.path();

    let mut registry = SkillRegistry::new(skills_dir);

    // 1. Add 1 valid skill
    let valid_dir = skills_dir.join("valid-skill");
    tokio::fs::create_dir_all(&valid_dir).await.unwrap();
    tokio::fs::write(
        valid_dir.join("SKILL.md"),
        "---\nname: Valid Skill\ndescription: I am valid\n---\n# Instructions",
    )
    .await
    .unwrap();

    // 2. Add an invalid skill (missing name)
    let invalid_dir = skills_dir.join("invalid-skill");
    tokio::fs::create_dir_all(&invalid_dir).await.unwrap();
    tokio::fs::write(
        invalid_dir.join("SKILL.md"),
        "---\ndescription: Missing name field\n---\n# Instructions",
    )
    .await
    .unwrap();

    // 3. Add a corrupted YAML file
    tokio::fs::write(
        skills_dir.join("corrupted.md"),
        "---\n[invalid yaml: : :{{\n---\n",
    )
    .await
    .unwrap();

    // 4. Add a plain notes file without frontmatter
    tokio::fs::write(
        skills_dir.join("notes.md"),
        "# Plain Notes\nNothing to see here.",
    )
    .await
    .unwrap();

    // Reload: Only the valid skill should be loaded into cache, others safely skipped
    let reloaded = registry
        .reload_skills()
        .await
        .expect("Hot reload should succeed");
    assert_eq!(reloaded.len(), 1);
    assert_eq!(registry.cached_skills().len(), 1);
    assert_eq!(registry.cached_skills()[0].id, "valid-skill");

    // Now fix the invalid skill dynamically at runtime
    tokio::fs::write(
        invalid_dir.join("SKILL.md"),
        "---\nname: Now Fixed Skill\ndescription: Fixed description\n---\n# Instructions",
    )
    .await
    .unwrap();

    // Reload again: Now both valid skills must be present!
    let reloaded_after_fix = registry
        .reload_skills()
        .await
        .expect("Hot reload should succeed");
    assert_eq!(reloaded_after_fix.len(), 2);
    assert_eq!(registry.cached_skills().len(), 2);
    assert!(registry.get_cached_skill("Now Fixed Skill").is_some());
}

#[tokio::test]
async fn test_security_path_traversal_on_hot_reloaded_registry() {
    let temp_guard = TempDirGuard::new("security");
    let skills_dir = temp_guard.path();

    let mut registry = SkillRegistry::new(skills_dir);
    registry.reload_skills().await.unwrap();

    // Path traversal attacks must be rejected
    assert!(registry.load_skill("../../etc/passwd").await.is_err());
    assert!(registry.load_skill("../sensitive").await.is_err());
    assert!(registry.load_skill("").await.is_err());
    assert!(registry.load_skill("   ").await.is_err());
    assert!(registry.load_skill("sub/dir").await.is_err());
    assert!(registry.load_skill("sub\\dir").await.is_err());
    assert!(registry.load_skill("null\0byte").await.is_err());
}
