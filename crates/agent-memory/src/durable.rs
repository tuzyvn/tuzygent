use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

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
    legacy_lessons_file: PathBuf,
}

/// Trích xuất danh sách token tìm kiếm từ mục tiêu (Goal query).
/// - Giữ lại các từ khóa kỹ thuật ngắn quan trọng (>= 2 ký tự): api, git, sql, bug, fix, db, ci, cd, k8s, ...
/// - Loại bỏ các stop words thông dụng trong tiếng Anh và tiếng Việt.
pub fn extract_query_tokens(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with",
        "by", "from", "as", "is", "are", "was", "were", "be", "been", "being", "have", "has",
        "had", "do", "does", "did", "can", "could", "should", "would", "this", "that", "these",
        "those", "into", "then", "them", "their", "there", "what", "which", "who", "when",
        "where", "why", "how", "all", "any", "both", "each", "few", "more", "most", "other",
        "some", "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too", "very",
        // Từ dừng phổ biến tiếng Việt
        "và", "hoặc", "nhưng", "trong", "trên", "tại", "cho", "của", "với", "bởi",
        "từ", "là", "được", "có", "này", "đó", "các", "những", "một", "thì", "mà", "nếu",
    ];

    query
        .to_lowercase()
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
                .to_string()
        })
        .filter(|w| {
            let char_count = w.chars().count();
            char_count >= 2 && !STOP_WORDS.contains(&w.as_str())
        })
        .collect()
}

/// Kiểm tra một bài học có khớp ngữ nghĩa với danh sách từ khóa không.
/// So khớp theo ranh giới token từ để tránh false positive (ví dụ 'port' không bị match nhầm 'important').
pub fn text_matches_keywords(text: &str, keywords: &[String]) -> bool {
    if keywords.is_empty() {
        return false;
    }
    let text_lower = text.to_lowercase();
    let tokens: HashSet<&str> = text_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    keywords.iter().any(|kw| {
        let kw_str = kw.as_str();
        tokens.contains(kw_str)
            || tokens.iter().any(|t| {
                // Hỗ trợ số nhiều / biến thể ngắn (e.g. port -> ports, task -> tasks)
                (t.len() > kw_str.len() && t.starts_with(kw_str) && t.len() - kw_str.len() <= 2)
                    || (kw_str.len() > t.len() && kw_str.starts_with(*t) && kw_str.len() - t.len() <= 2)
            })
    })
}

/// Phân tách các khối bài học trong MEMORY.md (hỗ trợ trọn vẹn cả bài học nhiều dòng)
pub fn parse_lesson_blocks(content: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current_block = String::new();

    for line in content.lines() {
        if line.trim_start().starts_with("- [Learned") {
            if !current_block.trim().is_empty() {
                blocks.push(current_block);
            }
            current_block = line.to_string();
        } else if !current_block.is_empty() {
            current_block.push('\n');
            current_block.push_str(line);
        }
    }
    if !current_block.trim().is_empty() {
        blocks.push(current_block);
    }
    blocks
}

impl DurableMemoryStore {
    pub fn new<P: AsRef<Path>>(workspace_dir: P) -> Self {
        let base = workspace_dir.as_ref().join("memory");
        let storage_dir = workspace_dir.as_ref().join("storage").join("harness_memory");
        let structured_lessons = storage_dir.join("structured_lessons.json");
        let legacy_lessons = storage_dir.join("lessons.json");
        Self {
            memory_file: base.join("MEMORY.md"),
            user_file: base.join("USER.md"),
            lessons_json_file: structured_lessons,
            legacy_lessons_file: legacy_lessons,
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
            let blocks = parse_lesson_blocks(&memory_content);
            let goal_words = extract_query_tokens(goal);

            let mut matched_lessons = Vec::new();
            let mut recent_lessons = Vec::new();

            for block in blocks.into_iter().rev() {
                if !goal_words.is_empty() && text_matches_keywords(&block, &goal_words) {
                    matched_lessons.push(block);
                } else if recent_lessons.len() < 8 {
                    recent_lessons.push(block);
                }
            }

            context.push_str("# DURABLE MEMORY & RELEVANT LESSONS:\n");
            if !matched_lessons.is_empty() {
                context.push_str("## High Relevance to Current Goal:\n");
                for l in matched_lessons.into_iter().take(10) {
                    context.push_str(&l);
                    context.push('\n');
                }
            }
            if !recent_lessons.is_empty() {
                context.push_str("## Recent Lessons Learned:\n");
                for l in recent_lessons.into_iter().take(6) {
                    context.push_str(&l);
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
        let entry = format!("- [Learned {}] {}\n", now, lesson.trim());

        let need_header = match fs::metadata(&self.memory_file).await {
            Ok(meta) => meta.len() == 0,
            Err(_) => true,
        };

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.memory_file)
            .await?;

        if need_header {
            file.write_all(b"# DURABLE MEMORY & LESSONS LEARNED\n\nCurated lessons and operational procedures discovered during autonomous runs:\n\n").await?;
        }

        file.write_all(entry.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    pub async fn load_structured_lessons(&self) -> Result<Vec<StructuredLesson>, std::io::Error> {
        if self.lessons_json_file.exists() {
            let content = fs::read_to_string(&self.lessons_json_file).await?;
            match serde_json::from_str::<Vec<StructuredLesson>>(&content) {
                Ok(lessons) => return Ok(lessons),
                Err(err) => {
                    // Backup corrupt file to prevent total loss
                    let ts = chrono::Utc::now().timestamp();
                    let corrupt_backup = self.lessons_json_file.with_extension(format!("corrupt.{}.json", ts));
                    let _ = fs::copy(&self.lessons_json_file, &corrupt_backup).await;
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Failed to parse structured lessons (backup saved to {:?}): {}", corrupt_backup, err),
                    ));
                }
            }
        }

        // Migration check: If new file does not exist, check legacy lessons.json
        if self.legacy_lessons_file.exists() {
            if let Ok(content) = fs::read_to_string(&self.legacy_lessons_file).await {
                if let Ok(legacy_lessons) = serde_json::from_str::<Vec<StructuredLesson>>(&content) {
                    // Save to new structured_lessons.json atomically
                    if let Some(parent) = self.lessons_json_file.parent() {
                        let _ = fs::create_dir_all(parent).await;
                    }
                    if let Ok(json_data) = serde_json::to_string_pretty(&legacy_lessons) {
                        let _ = fs::write(&self.lessons_json_file, json_data).await;
                    }
                    return Ok(legacy_lessons);
                }
            }
        }

        Ok(Vec::new())
    }

    pub async fn save_structured_lesson(
        &self,
        lesson: &StructuredLesson,
    ) -> Result<(), std::io::Error> {
        // 1. Lưu vào storage/harness_memory/structured_lessons.json qua atomic write (temp + rename)
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
        let json_data = serde_json::to_string_pretty(&lessons)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let temp_file = self.lessons_json_file.with_extension(format!("tmp.{}", uuid::Uuid::new_v4().simple()));
        fs::write(&temp_file, &json_data).await?;
        fs::rename(&temp_file, &self.lessons_json_file).await?;

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
        let entry = format!(
            "\n<!-- observed: {} | status: active -->\n{}\n",
            date_str,
            directive.trim()
        );

        let need_header = match fs::metadata(&self.user_file).await {
            Ok(meta) => meta.len() == 0,
            Err(_) => true,
        };

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.user_file)
            .await?;

        if need_header {
            file.write_all(b"# USER DIRECTIVES\n\nStable user preferences and operational directives:\n").await?;
        }

        file.write_all(entry.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }
}
