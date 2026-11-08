use agent_memory::skills::{
    frontmatter::{parse_frontmatter, validate_frontmatter, validate_skill_file},
    loader::{scan_and_validate_skills, scan_available_skills},
    registry::SkillRegistry,
    types::{SkillFrontmatter, SkillMetadata, SkillValidationError},
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("skills_decomp_{}_{}_{}", prefix, nanos, id));
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
async fn test_skills_submodule_structure_and_direct_exports(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDirGuard::new("modular");
    let skill_folder = temp.path().join("code-review");
    fs::create_dir_all(&skill_folder).await?;

    let valid_yaml =
        "---\nname: Code Review\ndescription: Reviews code quality\n---\n# Skill Details";
    fs::write(skill_folder.join("SKILL.md"), valid_yaml).await?;

    // 1. Verify types submodule
    let fm: SkillFrontmatter = validate_frontmatter(valid_yaml)?;
    assert_eq!(fm.name, "Code Review");
    assert_eq!(fm.description, "Reviews code quality");

    // 2. Verify frontmatter parser
    let (name, desc) = parse_frontmatter(valid_yaml);
    assert_eq!(name, "Code Review");
    assert_eq!(desc, "Reviews code quality");

    // 3. Verify loader submodule functions
    let available: Vec<SkillMetadata> = scan_available_skills(temp.path()).await?;
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].name, "Code Review");

    let reloaded: Vec<SkillMetadata> = scan_and_validate_skills(temp.path()).await?;
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].id, "code-review");

    // 4. Verify registry submodule
    let mut reg = SkillRegistry::new(temp.path());
    let reloaded_reg = reg.reload_skills().await?;
    assert_eq!(reloaded_reg.len(), 1);
    let loaded = reg.load_skill("code-review").await?;
    assert!(loaded.contains("# Skill Details"));

    // Verify error type from types submodule
    let invalid = "invalid frontmatter";
    let err = validate_frontmatter(invalid).unwrap_err();
    assert_eq!(err, SkillValidationError::MissingFrontmatter);

    Ok(())
}

#[tokio::test]
async fn test_validate_skill_file_reads_and_validates_frontmatter(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDirGuard::new("validate_file");
    let skill_file = temp.path().join("SKILL.md");
    fs::write(
        &skill_file,
        "---\nname: File Skill\ndescription: Validates frontmatter from disk\nversion: 1.2.3\nauthor: Test Author\ntags:\n  - testing\n  - skills\n---\n# Instructions\n",
    )
    .await?;

    let frontmatter = validate_skill_file(&skill_file).await?;

    assert_eq!(frontmatter.name, "File Skill");
    assert_eq!(frontmatter.description, "Validates frontmatter from disk");
    assert_eq!(frontmatter.version.as_deref(), Some("1.2.3"));
    assert_eq!(frontmatter.author.as_deref(), Some("Test Author"));
    assert_eq!(frontmatter.tags, vec!["testing", "skills"]);
    Ok(())
}

#[tokio::test]
async fn test_validate_skill_file_reports_read_error_for_missing_path() {
    let temp = TempDirGuard::new("validate_file_missing");
    let missing_file = temp.path().join("missing-SKILL.md");

    let error = validate_skill_file(&missing_file)
        .await
        .expect_err("a missing skill file should fail to read");

    assert!(
        matches!(error, SkillValidationError::InvalidYaml(ref message) if !message.is_empty()),
        "read failures should be reported as InvalidYaml with the I/O error: {error:?}"
    );
}

#[test]
fn test_frontmatter_requires_standalone_closing_delimiter() {
    let malformed = "---\nname: Almost Valid\ndescription: Closing marker has trailing text\n--- trailing text\n# Skill Details";

    assert_eq!(
        validate_frontmatter(malformed),
        Err(SkillValidationError::MissingFrontmatter),
        "a closing delimiter must occupy its own line"
    );
}
