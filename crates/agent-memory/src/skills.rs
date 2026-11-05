use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tokio::fs;

#[allow(clippy::expect_used)]
static FRONTMATTER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\A---\r?\n(.*?\r?\n)---").expect("Failed to compile FRONTMATTER_RE regex")
});

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SkillValidationError {
    #[error("Missing frontmatter delimiters ('---')")]
    MissingFrontmatter,
    #[error("Failed to parse YAML frontmatter: {0}")]
    InvalidYaml(String),
    #[error("Missing or empty required field 'name' in frontmatter")]
    MissingName,
    #[error("Missing or empty required field 'description' in frontmatter")]
    MissingDescription,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub content: String,
    #[serde(default)]
    pub frontmatter: Option<SkillFrontmatter>,
}

#[derive(Debug, Clone)]
pub struct SkillRegistry {
    skills_dir: PathBuf,
    skills_cache: Vec<SkillMetadata>,
}

impl SkillRegistry {
    pub fn new<P: AsRef<Path>>(skills_dir: P) -> Self {
        Self {
            skills_dir: skills_dir.as_ref().to_path_buf(),
            skills_cache: Vec::new(),
        }
    }

    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    pub fn cached_skills(&self) -> &[SkillMetadata] {
        &self.skills_cache
    }

    pub fn get_cached_skill(&self, identifier: &str) -> Option<&SkillMetadata> {
        let trimmed = identifier.trim();
        self.skills_cache
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(trimmed) || s.id.eq_ignore_ascii_case(trimmed))
    }

    /// Xác thực schema của YAML frontmatter trong nội dung kỹ năng
    pub fn validate_frontmatter(content: &str) -> Result<SkillFrontmatter, SkillValidationError> {
        let trimmed = content.trim_start_matches('\u{feff}').trim_start();
        if !trimmed.starts_with("---") {
            return Err(SkillValidationError::MissingFrontmatter);
        }

        let caps = FRONTMATTER_RE
            .captures(trimmed)
            .ok_or(SkillValidationError::MissingFrontmatter)?;
        let yaml_block = caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or(SkillValidationError::MissingFrontmatter)?;

        let yaml_val: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml_block)
            .map_err(|e| SkillValidationError::InvalidYaml(e.to_string()))?;

        let name = yaml_val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            return Err(SkillValidationError::MissingName);
        }

        let description = yaml_val
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if description.is_empty() {
            return Err(SkillValidationError::MissingDescription);
        }

        let version = yaml_val
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let author = yaml_val
            .get("author")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let tags = yaml_val
            .get("tags")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(SkillFrontmatter {
            name: name.to_string(),
            description: description.to_string(),
            version,
            author,
            tags,
        })
    }

    /// Xác thực schema của một file kỹ năng trực tiếp từ đường dẫn đĩa
    pub async fn validate_skill_file<P: AsRef<Path>>(
        path: P,
    ) -> Result<SkillFrontmatter, SkillValidationError> {
        let content = fs::read_to_string(path)
            .await
            .map_err(|e| SkillValidationError::InvalidYaml(e.to_string()))?;
        Self::validate_frontmatter(&content)
    }

    /// Trích xuất Frontmatter chuẩn YAML `--- ... ---` theo quy chuẩn Multica
    pub fn parse_frontmatter(content: &str) -> (String, String) {
        let trimmed = content.trim_start_matches('\u{feff}').trim_start();
        if !trimmed.starts_with("---") {
            return ("".to_string(), "".to_string());
        }

        if let Some(caps) = FRONTMATTER_RE.captures(trimmed) {
            if let Some(yaml_block) = caps.get(1).map(|m| m.as_str()) {
                if let Ok(yaml_map) = serde_yaml_ng::from_str::<serde_json::Value>(yaml_block) {
                    let name = yaml_map
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let desc = yaml_map
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    return (name, desc);
                }
            }
        }
        ("".to_string(), "".to_string())
    }

    pub async fn list_available_skills(&self) -> Result<Vec<SkillMetadata>, std::io::Error> {
        let mut skills = Vec::new();
        if !self.skills_dir.exists() {
            return Ok(skills);
        }

        let mut entries = fs::read_dir(&self.skills_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if skill_file.exists() && skill_file.is_file() {
                    let content = fs::read_to_string(&skill_file).await.unwrap_or_default();
                    let (name, desc) = Self::parse_frontmatter(&content);
                    let folder_name = entry.file_name().to_string_lossy().to_string();
                    let final_name = if !name.is_empty() {
                        name
                    } else {
                        folder_name.clone()
                    };
                    let final_desc = if !desc.is_empty() {
                        desc
                    } else {
                        content
                            .lines()
                            .next()
                            .unwrap_or("No description")
                            .to_string()
                    };

                    skills.push(SkillMetadata {
                        id: folder_name,
                        name: final_name,
                        description: final_desc,
                        path: skill_file,
                        frontmatter: Self::validate_frontmatter(&content).ok(),
                        content,
                    });
                }
            } else if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if !file_name.eq_ignore_ascii_case("README.md") {
                    let content = fs::read_to_string(&path).await.unwrap_or_default();
                    let (name, desc) = Self::parse_frontmatter(&content);
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| file_name.clone());
                    let final_name = if !name.is_empty() { name } else { stem.clone() };
                    let final_desc = if !desc.is_empty() {
                        desc
                    } else {
                        content
                            .lines()
                            .next()
                            .unwrap_or("No description")
                            .to_string()
                    };

                    if !skills.iter().any(|s| s.id == stem) {
                        skills.push(SkillMetadata {
                            id: stem,
                            name: final_name,
                            description: final_desc,
                            path,
                            frontmatter: Self::validate_frontmatter(&content).ok(),
                            content,
                        });
                    }
                }
            }
        }
        skills.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(skills)
    }

    /// Dynamic Skill Hot-Reloading: Quét lại thư mục trên đĩa, xác thực schema,
    /// nạp và cập nhật bộ nhớ cache nội tại của tiến trình runner/daemon.
    pub async fn reload_skills(&mut self) -> Result<Vec<SkillMetadata>, std::io::Error> {
        let mut reloaded = Vec::new();
        if !self.skills_dir.exists() {
            self.skills_cache.clear();
            return Ok(reloaded);
        }

        let mut entries = fs::read_dir(&self.skills_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if skill_file.exists() && skill_file.is_file() {
                    if let Ok(content) = fs::read_to_string(&skill_file).await {
                        if let Ok(fm) = Self::validate_frontmatter(&content) {
                            let folder_name = entry.file_name().to_string_lossy().to_string();
                            reloaded.push(SkillMetadata {
                                id: folder_name,
                                name: fm.name.clone(),
                                description: fm.description.clone(),
                                path: skill_file,
                                content,
                                frontmatter: Some(fm),
                            });
                        }
                    }
                }
            } else if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if !file_name.eq_ignore_ascii_case("README.md") {
                    if let Ok(content) = fs::read_to_string(&path).await {
                        if let Ok(fm) = Self::validate_frontmatter(&content) {
                            let stem = path
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| file_name.clone());
                            if !reloaded.iter().any(|s| s.id == stem) {
                                reloaded.push(SkillMetadata {
                                    id: stem,
                                    name: fm.name.clone(),
                                    description: fm.description.clone(),
                                    path,
                                    content,
                                    frontmatter: Some(fm),
                                });
                            }
                        }
                    }
                }
            }
        }

        reloaded.sort_by(|a, b| a.id.cmp(&b.id));
        self.skills_cache = reloaded.clone();
        Ok(reloaded)
    }

    /// Nạp nội dung một kỹ năng từ tên folder hoặc tên khai báo trong metadata.
    /// Có cơ chế phòng chống Path Traversal nghiêm ngặt và ưu tiên tìm trong bộ nhớ đệm cache.
    pub async fn load_skill(&self, skill_name: &str) -> Result<String, std::io::Error> {
        let trimmed_name = skill_name.trim();
        // Kiểm tra an ninh chống tấn công đường dẫn duyệt thư mục
        if trimmed_name.is_empty()
            || trimmed_name.contains("..")
            || trimmed_name.contains('/')
            || trimmed_name.contains('\\')
            || trimmed_name.contains('\0')
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid or unsafe skill name: '{}'", skill_name),
            ));
        }

        // 1. Kiểm tra cache trước nếu đã nạp
        if let Some(cached) = self.get_cached_skill(trimmed_name) {
            return Ok(cached.content.clone());
        }

        // 2. Kiểm tra trực tiếp trên đĩa theo thư mục <trimmed_name>/SKILL.md
        let direct_file = self.skills_dir.join(trimmed_name).join("SKILL.md");
        if direct_file.exists() && direct_file.is_file() {
            return fs::read_to_string(direct_file).await;
        }

        // 3. Kiểm tra trực tiếp trên đĩa theo file <trimmed_name>.md
        let direct_md = self.skills_dir.join(format!("{}.md", trimmed_name));
        if direct_md.exists() && direct_md.is_file() {
            return fs::read_to_string(direct_md).await;
        }

        let direct_file_exact = self.skills_dir.join(trimmed_name);
        if direct_file_exact.is_file() {
            return fs::read_to_string(direct_file_exact).await;
        }

        // 4. Nếu không khớp trực tiếp tên thư mục/file, tìm theo trường `name` hoặc `id` trong danh sách skills
        let all_skills = self.list_available_skills().await?;
        if let Some(matched) = all_skills.into_iter().find(|s| {
            s.name.eq_ignore_ascii_case(trimmed_name) || s.id.eq_ignore_ascii_case(trimmed_name)
        }) {
            return fs::read_to_string(matched.path).await;
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Skill '{}' not found in {}",
                trimmed_name,
                self.skills_dir.display()
            ),
        ))
    }
}

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
