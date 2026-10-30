use std::path::{Path, PathBuf};
use tokio::fs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredLesson {
    pub id: String,
    pub timestamp: String,
    pub task_title: String,
    pub trigger_error: Option<String>,
    pub remediation_applied: Option<String>,
    pub general_insight: String,
    pub success: bool,
}

pub struct DurableMemoryStore {
    memory_file: PathBuf,
    user_file: PathBuf,
    lessons_json_file: PathBuf,
}

impl DurableMemoryStore {
    pub fn new<P: AsRef<Path>>(workspace_dir: P) -> Self {
        let base = workspace_dir.as_ref().join("memory");
        let storage_lessons = workspace_dir
            .as_ref()
            .join("storage")
            .join("harness_memory")
            .join("lessons.json");
        Self {
            memory_file: base.join("MEMORY.md"),
            user_file: base.join("USER.md"),
            lessons_json_file: storage_lessons,
        }
    }

    pub async fn load_long_term_context(&self) -> Result<String, std::io::Error> {
        self.load_relevant_context("").await
    }

    pub async fn load_relevant_context(&self, goal: &str) -> Result<String, std::io::Error> {
        let mut context = String::new();
        if self.user_file.exists() {
            let user_content = fs::read_to_string(&self.user_file).await?;
            context.push_str("# USER DIRECTIVES:\n");
            context.push_str(&user_content);
            context.push_str("\n\n");
        }
        if self.memory_file.exists() {
            let memory_content = fs::read_to_string(&self.memory_file).await?;
            let lines: Vec<&str> = memory_content.lines().collect();
            let goal_words: Vec<String> = goal
                .to_lowercase()
                .split_whitespace()
                .filter(|w| w.len() > 3)
                .map(|w| w.to_string())
                .collect();

            let mut matched_lessons = Vec::new();
            let mut recent_lessons = Vec::new();

            for line in lines.iter().rev() {
                if line.trim().starts_with("- [Learned") {
                    let line_lower = line.to_lowercase();
                    if !goal_words.is_empty() && goal_words.iter().any(|w| line_lower.contains(w)) {
                        matched_lessons.push(*line);
                    } else if recent_lessons.len() < 8 {
                        recent_lessons.push(*line);
                    }
                }
            }

            context.push_str("# DURABLE MEMORY & RELEVANT LESSONS:\n");
            if !matched_lessons.is_empty() {
                context.push_str("## High Relevance to Current Goal:\n");
                for l in matched_lessons.into_iter().take(10) {
                    context.push_str(l);
                    context.push('\n');
                }
            }
            if !recent_lessons.is_empty() {
                context.push_str("## Recent Lessons Learned:\n");
                for l in recent_lessons.into_iter().take(6) {
                    context.push_str(l);
                    context.push('\n');
                }
            }
        }
        Ok(context)
    }

    pub async fn save_lesson(&self, lesson: &str) -> Result<(), std::io::Error> {
        if let Some(parent) = self.memory_file.parent() {
            fs::create_dir_all(parent).await?;
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut current = if self.memory_file.exists() {
            fs::read_to_string(&self.memory_file).await?
        } else {
            "# DURABLE MEMORY & LESSONS LEARNED\n\nCurated lessons and operational procedures discovered during autonomous runs:\n".to_string()
        };
        current.push_str(&format!("\n- [Learned {}] {}", now, lesson));
        fs::write(&self.memory_file, current).await
    }

    pub async fn load_structured_lessons(&self) -> Result<Vec<StructuredLesson>, std::io::Error> {
        if !self.lessons_json_file.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.lessons_json_file).await?;
        let lessons: Vec<StructuredLesson> = serde_json::from_str(&content).unwrap_or_default();
        Ok(lessons)
    }

    pub async fn save_structured_lesson(&self, lesson: &StructuredLesson) -> Result<(), std::io::Error> {
        // 1. Lưu vào storage/harness_memory/lessons.json (cho Dashboard & UI hiển thị)
        if let Some(parent) = self.lessons_json_file.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut lessons = self.load_structured_lessons().await.unwrap_or_default();
        // Giữ tối đa 100 bài học gần nhất, bài học mới xếp lên đầu
        lessons.retain(|l| l.id != lesson.id);
        lessons.insert(0, lesson.clone());
        if lessons.len() > 100 {
            lessons.truncate(100);
        }
        let json_data = serde_json::to_string_pretty(&lessons).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        fs::write(&self.lessons_json_file, json_data).await?;

        // 2. Đồng bộ tóm tắt có gắn nhãn ngữ nghĩa vào memory/MEMORY.md (cho Prompt Context)
        let status_tag = if lesson.success { "SUCCESS" } else { "FAILURE" };
        let md_line = if let Some(ref err) = lesson.trigger_error {
            let remedy = lesson.remediation_applied.as_deref().unwrap_or("None");
            format!(
                "[{}] Goal '{}': Error '{}' solved by '{}' -> Insight: {}",
                status_tag, lesson.task_title, err, remedy, lesson.general_insight
            )
        } else {
            format!(
                "[{}] Goal '{}': {}",
                status_tag, lesson.task_title, lesson.general_insight
            )
        };
        self.save_lesson(&md_line).await?;

        Ok(())
    }

    pub async fn save_user_directive(&self, directive: &str) -> Result<(), std::io::Error> {
        if let Some(parent) = self.user_file.parent() {
            fs::create_dir_all(parent).await?;
        }
        let date_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut current = if self.user_file.exists() {
            fs::read_to_string(&self.user_file).await?
        } else {
            "# USER DIRECTIVES\n\nStable user preferences and operational directives:\n".to_string()
        };
        current.push_str(&format!("\n<!-- observed: {} | status: active -->\n{}\n", date_str, directive));
        fs::write(&self.user_file, current).await
    }
}
