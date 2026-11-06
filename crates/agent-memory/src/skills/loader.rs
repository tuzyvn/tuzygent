use std::path::Path;
use tokio::fs;

use crate::skills::frontmatter::{parse_frontmatter, validate_frontmatter};
use crate::skills::types::SkillMetadata;

/// Quét thư mục skills để phát hiện tất cả file kỹ năng
pub async fn scan_available_skills(skills_dir: &Path) -> Result<Vec<SkillMetadata>, std::io::Error> {
    let mut skills = Vec::new();
    if !skills_dir.exists() {
        return Ok(skills);
    }

    let mut entries = fs::read_dir(skills_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            let skill_file = path.join("SKILL.md");
            if skill_file.exists() && skill_file.is_file() {
                let content = fs::read_to_string(&skill_file).await.unwrap_or_default();
                let (name, desc) = parse_frontmatter(&content);
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
                    frontmatter: validate_frontmatter(&content).ok(),
                    content,
                });
            }
        } else if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !file_name.eq_ignore_ascii_case("README.md") {
                let content = fs::read_to_string(&path).await.unwrap_or_default();
                let (name, desc) = parse_frontmatter(&content);
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
                        frontmatter: validate_frontmatter(&content).ok(),
                        content,
                    });
                }
            }
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
}

/// Quét và chỉ nạp các kỹ năng có frontmatter hợp lệ cho hot-reload
pub async fn scan_and_validate_skills(
    skills_dir: &Path,
) -> Result<Vec<SkillMetadata>, std::io::Error> {
    let mut reloaded = Vec::new();
    if !skills_dir.exists() {
        return Ok(reloaded);
    }

    let mut entries = fs::read_dir(skills_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            let skill_file = path.join("SKILL.md");
            if skill_file.exists() && skill_file.is_file() {
                if let Ok(content) = fs::read_to_string(&skill_file).await {
                    if let Ok(fm) = validate_frontmatter(&content) {
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
                    if let Ok(fm) = validate_frontmatter(&content) {
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
    Ok(reloaded)
}
