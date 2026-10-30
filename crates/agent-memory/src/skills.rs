use std::path::{Path, PathBuf};
use tokio::fs;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub content: String,
}

pub struct SkillRegistry {
    skills_dir: PathBuf,
}

impl SkillRegistry {
    pub fn new<P: AsRef<Path>>(skills_dir: P) -> Self {
        Self {
            skills_dir: skills_dir.as_ref().to_path_buf(),
        }
    }

    /// Trích xuất Frontmatter chuẩn YAML `--- ... ---` theo quy chuẩn Multica
    pub fn parse_frontmatter(content: &str) -> (String, String) {
        if !content.starts_with("---") {
            return ("".to_string(), "".to_string());
        }

        let re = Regex::new(r"(?s)\A---\r?\n(.*?\r?\n)---").unwrap();
        if let Some(caps) = re.captures(content) {
            let yaml_block = &caps[1];
            if let Ok(yaml_map) = serde_yaml_ng::from_str::<serde_json::Value>(yaml_block) {
                let name = yaml_map.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let desc = yaml_map.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                return (name, desc);
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
                if skill_file.exists() {
                    let content = fs::read_to_string(&skill_file).await.unwrap_or_default();
                    let (name, desc) = Self::parse_frontmatter(&content);
                    let final_name = if !name.is_empty() {
                        name
                    } else {
                        entry.file_name().to_string_lossy().to_string()
                    };
                    let final_desc = if !desc.is_empty() {
                        desc
                    } else {
                        content.lines().next().unwrap_or("No description").to_string()
                    };

                    skills.push(SkillMetadata {
                        name: final_name,
                        description: final_desc,
                        path: skill_file,
                        content,
                    });
                }
            }
        }
        Ok(skills)
    }

    pub async fn load_skill(&self, skill_name: &str) -> Result<String, std::io::Error> {
        let skill_file = self.skills_dir.join(skill_name).join("SKILL.md");
        fs::read_to_string(skill_file).await
    }
}
