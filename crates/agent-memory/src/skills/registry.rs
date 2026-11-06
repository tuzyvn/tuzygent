use std::path::{Path, PathBuf};
use tokio::fs;

use crate::skills::frontmatter::{parse_frontmatter, validate_frontmatter, validate_skill_file};
use crate::skills::loader::{scan_and_validate_skills, scan_available_skills};
use crate::skills::types::{SkillFrontmatter, SkillMetadata, SkillValidationError};

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
        validate_frontmatter(content)
    }

    /// Xác thực schema của một file kỹ năng trực tiếp từ đường dẫn đĩa
    pub async fn validate_skill_file<P: AsRef<Path>>(
        path: P,
    ) -> Result<SkillFrontmatter, SkillValidationError> {
        validate_skill_file(path).await
    }

    /// Trích xuất Frontmatter chuẩn YAML `--- ... ---` theo quy chuẩn Multica
    pub fn parse_frontmatter(content: &str) -> (String, String) {
        parse_frontmatter(content)
    }

    pub async fn list_available_skills(&self) -> Result<Vec<SkillMetadata>, std::io::Error> {
        scan_available_skills(&self.skills_dir).await
    }

    /// Dynamic Skill Hot-Reloading: Quét lại thư mục trên đĩa, xác thực schema,
    /// nạp và cập nhật bộ nhớ cache nội tại của tiến trình runner/daemon.
    pub async fn reload_skills(&mut self) -> Result<Vec<SkillMetadata>, std::io::Error> {
        let reloaded = scan_and_validate_skills(&self.skills_dir).await?;
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
