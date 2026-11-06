use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;
use tokio::fs;

use crate::skills::types::{SkillFrontmatter, SkillValidationError};

#[allow(clippy::expect_used)]
pub static FRONTMATTER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\A---\r?\n(.*?\r?\n)---").expect("Failed to compile FRONTMATTER_RE regex")
});

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
    validate_frontmatter(&content)
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
