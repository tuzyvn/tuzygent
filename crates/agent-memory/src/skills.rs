pub mod frontmatter;
pub mod loader;
pub mod registry;
pub mod types;

pub use frontmatter::{parse_frontmatter, validate_frontmatter, validate_skill_file, FRONTMATTER_RE};
pub use loader::{scan_and_validate_skills, scan_available_skills};
pub use registry::SkillRegistry;
pub use types::{SkillFrontmatter, SkillMetadata, SkillValidationError};


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frontmatter_regex_captures_valid_yaml() -> Result<(), &'static str> {
        let content =
            "---\nname: git-analyzer\ndescription: Analyzes git repositories\n---\n# Body";
        let caps = FRONTMATTER_RE
            .captures(content)
            .ok_or("Expected frontmatter regex captures to match")?;
        let yaml_block = caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or("Expected capture group 1")?;
        assert_eq!(
            yaml_block,
            "name: git-analyzer\ndescription: Analyzes git repositories\n"
        );
        Ok(())
    }

    #[test]
    fn test_frontmatter_regex_handles_crlf() -> Result<(), &'static str> {
        let content =
            "---\r\nname: crlf-agent\r\ndescription: Windows line endings\r\n---\r\n# Body";
        let caps = FRONTMATTER_RE
            .captures(content)
            .ok_or("Expected frontmatter regex captures to match")?;
        let yaml_block = caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or("Expected capture group 1")?;
        assert_eq!(
            yaml_block,
            "name: crlf-agent\r\ndescription: Windows line endings\r\n"
        );
        Ok(())
    }

    #[test]
    fn test_regex_captures_safe_handling_none_and_missing_groups_without_unwrap() {
        // Case 1: Regex captures returns None (non-matching content)
        let invalid_content = "Not a frontmatter\ncontent";
        let caps = FRONTMATTER_RE.captures(invalid_content);
        assert!(caps.is_none());

        // Safe extraction using and_then + get(1) returns None without panicking or calling unwrap()
        let yaml_block = caps.as_ref().and_then(|c| c.get(1)).map(|m| m.as_str());
        assert_eq!(yaml_block, None);

        // Case 2: Regex captures returns Some, but group does not exist (out of range group index)
        let valid_content = "---\nname: safe-agent\ndescription: Safe regex test\n---\n# Body";
        let caps = FRONTMATTER_RE.captures(valid_content);
        assert!(caps.is_some());

        let matched = caps.as_ref();
        // Existing group 1 returns Some
        let group_1 = matched.and_then(|c| c.get(1)).map(|m| m.as_str());
        assert_eq!(
            group_1,
            Some("name: safe-agent\ndescription: Safe regex test\n")
        );

        // Non-existent group 99 returns None safely without panicking or calling unwrap()
        let missing_group = matched.and_then(|c| c.get(99)).map(|m| m.as_str());
        assert_eq!(missing_group, None);

        // Case 3: parse_frontmatter execution path handling None captures safely
        let (name1, desc1) = SkillRegistry::parse_frontmatter(invalid_content);
        assert_eq!(name1, "");
        assert_eq!(desc1, "");

        let (name2, desc2) = SkillRegistry::parse_frontmatter("---\nunterminated frontmatter");
        assert_eq!(name2, "");
        assert_eq!(desc2, "");
    }

    #[test]
    fn test_frontmatter_regex_rejects_markdown_without_frontmatter() {
        // Plain markdown heading
        assert!(FRONTMATTER_RE
            .captures("# Normal Heading\nSome content")
            .is_none());

        // Markdown with horizontal rules in the middle
        assert!(FRONTMATTER_RE
            .captures("# Title\n\n---\n\nBody content")
            .is_none());

        // Empty string and whitespace only
        assert!(FRONTMATTER_RE.captures("").is_none());
        assert!(FRONTMATTER_RE.captures("   \n\n   ").is_none());

        // Dash line but not delimiter format
        assert!(FRONTMATTER_RE
            .captures("--- Not a valid delimiter\ncontent")
            .is_none());

        // Missing closing delimiter
        assert!(FRONTMATTER_RE
            .captures("---\nname: unclosed-skill\ndescription: no closing")
            .is_none());
    }

    #[test]
    fn test_parse_frontmatter_valid_yaml() {
        let content = r#"---
name: Code Review Expert
description: Automated static analysis and code smell reviewer
---

# Instructions
Inspect code for issues.
"#;
        let (name, desc) = SkillRegistry::parse_frontmatter(content);
        assert_eq!(name, "Code Review Expert");
        assert_eq!(desc, "Automated static analysis and code smell reviewer");
    }

    #[test]
    fn test_parse_frontmatter_with_utf8_bom_and_leading_whitespace() {
        let content =
            "\u{feff}  \n---\nname: bom-skill\ndescription: Works with BOM\n---\n# Content";
        let (name, desc) = SkillRegistry::parse_frontmatter(content);
        assert_eq!(name, "bom-skill");
        assert_eq!(desc, "Works with BOM");
    }

    #[test]
    fn test_parse_frontmatter_markdown_without_frontmatter() {
        // Standard markdown document
        let markdown = "# Rust Architecture\n\nThis document describes the system architecture.";
        let (name, desc) = SkillRegistry::parse_frontmatter(markdown);
        assert_eq!(name, "");
        assert_eq!(desc, "");

        // Markdown with horizontal rule later in content
        let hr_markdown = "Introduction\n\n---\n\nSection 2";
        let (name, desc) = SkillRegistry::parse_frontmatter(hr_markdown);
        assert_eq!(name, "");
        assert_eq!(desc, "");

        // Empty markdown
        let (name, desc) = SkillRegistry::parse_frontmatter("");
        assert_eq!(name, "");
        assert_eq!(desc, "");
    }

    #[test]
    fn test_parse_frontmatter_invalid_yaml_or_missing_fields() {
        // Corrupted YAML
        let corrupt = "---\n[invalid yaml: : :{{{\n---\n# Content";
        let (name, desc) = SkillRegistry::parse_frontmatter(corrupt);
        assert_eq!(name, "");
        assert_eq!(desc, "");

        // Missing description
        let only_name = "---\nname: only-name-skill\n---\n";
        let (name, desc) = SkillRegistry::parse_frontmatter(only_name);
        assert_eq!(name, "only-name-skill");
        assert_eq!(desc, "");
    }
}
