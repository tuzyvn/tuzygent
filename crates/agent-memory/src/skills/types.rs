use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
