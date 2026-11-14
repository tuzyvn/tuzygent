pub mod audit;
pub mod benchmark;
pub mod cache;
pub mod fault;
pub mod resilience;
#[cfg(test)]
mod tests;

pub use benchmark::{HarnessBenchmarkHarness, HarnessBenchmarkStats};
pub use cache::HarnessLessonCache;
pub use fault::{
    is_retryable_harness_error, simulate_harness_fault, HarnessError, HarnessFault,
    HarnessFaultPolicy,
};
pub use resilience::{ResilientHarnessMemory, ResilientHarnessMemoryStore};

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// Bài học kinh nghiệm tổng quát được trích xuất từ các chu trình Propose-Probe-Compose của Harness²
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessLesson {
    pub id: String,
    pub node_type: String,
    pub identifier: String,
    pub error_pattern: String,
    pub remediation_strategy: String,
    pub success_count: u32,
    pub created_at: String,
    pub last_applied_at: Option<String>,
}

/// Thư viện lưu trữ Harness Memory dài hạn
#[derive(Debug, Clone)]
pub struct HarnessMemoryStore {
    memory_path: PathBuf,
    legacy_path: PathBuf,
    in_memory: Option<Arc<tokio::sync::RwLock<Vec<HarnessLesson>>>>,
}

impl HarnessMemoryStore {
    pub fn memory() -> Self {
        Self {
            memory_path: PathBuf::from(":memory:"),
            legacy_path: PathBuf::from(":memory:"),
            in_memory: Some(Arc::new(tokio::sync::RwLock::new(Vec::new()))),
        }
    }

    pub fn new(workspace_dir: &str) -> Self {
        if workspace_dir == ":memory:" {
            return Self::memory();
        }
        let memory_dir = Path::new(workspace_dir)
            .join("storage")
            .join("harness_memory");
        let _ = std::fs::create_dir_all(&memory_dir);
        let memory_path = memory_dir.join("harness_lessons.json");
        let legacy_path = memory_dir.join("lessons.json");
        Self {
            memory_path,
            legacy_path,
            in_memory: None,
        }
    }

    pub fn memory_path(&self) -> &Path {
        &self.memory_path
    }

    pub fn legacy_path(&self) -> &Path {
        &self.legacy_path
    }

    /// Đọc tất cả các bài học kinh nghiệm đã tích lũy
    pub async fn load_lessons(&self) -> Vec<HarnessLesson> {
        match self.try_load_lessons().await {
            Ok(lessons) => lessons,
            Err(err) => {
                tracing::error!("Cannot load harness lessons: {err:#}");
                Vec::new()
            }
        }
    }

    pub async fn try_load_lessons(&self) -> Result<Vec<HarnessLesson>> {
        if let Some(mem) = &self.in_memory {
            return Ok(mem.read().await.clone());
        }

        if self.memory_path.exists() {
            match tokio::fs::read(&self.memory_path).await {
                Ok(bytes) => match serde_json::from_slice::<Vec<HarnessLesson>>(&bytes) {
                    Ok(lessons) => return Ok(lessons),
                    Err(err) => {
                        let ts = chrono::Utc::now().timestamp();
                        let corrupt = self
                            .memory_path
                            .with_extension(format!("corrupt.{}.json", ts));
                        let _ = tokio::fs::copy(&self.memory_path, &corrupt).await;
                        tracing::error!(
                            "Harness lessons corrupt, backup saved to {:?}: {}",
                            corrupt,
                            err
                        );
                        return Err(anyhow!("Invalid harness lessons: {}", err));
                    }
                },
                Err(err) => return Err(err.into()),
            }
        }

        // Migration from legacy lessons.json if present and matches HarnessLesson schema
        if self.legacy_path.exists() {
            if let Ok(bytes) = tokio::fs::read(&self.legacy_path).await {
                if let Ok(lessons) = serde_json::from_slice::<Vec<HarnessLesson>>(&bytes) {
                    return Ok(lessons);
                }
            }
        }

        Ok(Vec::new())
    }

    /// Ghi nhận hoặc tăng điểm thành công cho bài học kinh nghiệm
    pub async fn record_lesson(
        &self,
        node_type: &str,
        identifier: &str,
        error_pattern: &str,
        remediation_strategy: &str,
    ) -> Result<HarnessLesson> {
        if let Some(mem) = &self.in_memory {
            let mut lessons = mem.write().await;
            let now_str = Utc::now().to_rfc3339();

            let normalized_incoming = crate::sentinel::normalize_signature_text(error_pattern);

            if let Some(existing) = lessons.iter_mut().find(|l| {
                l.node_type == node_type
                    && l.identifier == identifier
                    && (l.error_pattern.contains(error_pattern)
                        || error_pattern.contains(&l.error_pattern)
                        || (!normalized_incoming.is_empty()
                            && crate::sentinel::normalize_signature_text(&l.error_pattern)
                                == normalized_incoming))
            }) {
                existing.success_count += 1;
                existing.last_applied_at = Some(now_str);
                return Ok(existing.clone());
            }

            let uuid_str = Uuid::new_v4().to_string();
            let short_id = uuid_str.get(..8).unwrap_or(&uuid_str);
            let new_lesson = HarnessLesson {
                id: format!("les-{}", short_id),
                node_type: node_type.to_string(),
                identifier: identifier.to_string(),
                error_pattern: error_pattern.to_string(),
                remediation_strategy: remediation_strategy.to_string(),
                success_count: 1,
                created_at: now_str.clone(),
                last_applied_at: Some(now_str),
            };

            lessons.push(new_lesson.clone());
            return Ok(new_lesson);
        }

        if let Some(parent) = self.memory_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let _lock = agent_memory::lock_memory_file(&self.memory_path).await?;
        let mut lessons = self.try_load_lessons().await?;
        let now_str = Utc::now().to_rfc3339();

        // Kiểm tra xem bài học tương tự đã có chưa
        let normalized_incoming = crate::sentinel::normalize_signature_text(error_pattern);
        if let Some(existing) = lessons.iter_mut().find(|l| {
            l.node_type == node_type
                && l.identifier == identifier
                && (l.error_pattern.contains(error_pattern)
                    || error_pattern.contains(&l.error_pattern)
                    || (!normalized_incoming.is_empty()
                        && crate::sentinel::normalize_signature_text(&l.error_pattern)
                            == normalized_incoming))
        }) {
            existing.success_count += 1;
            existing.last_applied_at = Some(now_str);
            let saved = existing.clone();
            self.save_all(&lessons).await?;
            return Ok(saved);
        }

        let uuid_str = Uuid::new_v4().to_string();
        let short_id = uuid_str.get(..8).unwrap_or(&uuid_str);
        let new_lesson = HarnessLesson {
            id: format!("les-{}", short_id),
            node_type: node_type.to_string(),
            identifier: identifier.to_string(),
            error_pattern: error_pattern.to_string(),
            remediation_strategy: remediation_strategy.to_string(),
            success_count: 1,
            created_at: now_str.clone(),
            last_applied_at: Some(now_str),
        };

        lessons.push(new_lesson.clone());
        self.save_all(&lessons).await?;
        Ok(new_lesson)
    }

    /// Tìm các bài học kinh nghiệm liên quan trước khi chạy Node
    pub async fn find_relevant_lessons(
        &self,
        node_type: &str,
        identifier: &str,
    ) -> Vec<HarnessLesson> {
        let lessons = self.load_lessons().await;
        lessons
            .into_iter()
            .filter(|l| {
                l.node_type == node_type && (l.identifier == identifier || l.identifier == "*")
            })
            .collect()
    }

    async fn save_all(&self, lessons: &[HarnessLesson]) -> Result<()> {
        if let Some(parent) = self.memory_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| anyhow!("Không tạo được thư mục harness_memory: {}", e))?;
        }
        let json_data = serde_json::to_string_pretty(lessons)
            .map_err(|e| anyhow!("Lỗi serialize harness lessons: {}", e))?;

        let temp_path = self
            .memory_path
            .with_extension(format!("tmp.{}", Uuid::new_v4().simple()));
        if let Err(e) = tokio::fs::write(&temp_path, &json_data).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(anyhow!("Không ghi được file tạm harness lessons: {}", e));
        }
        if let Err(e) = tokio::fs::rename(&temp_path, &self.memory_path).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(anyhow!(
                "Không thể rename atomic file harness lessons: {}",
                e
            ));
        }
        Ok(())
    }
}

/// Bộ máy đệ quy Propose-Probe-Compose theo chuẩn Harness²
pub struct HarnessSelfHealer;

impl HarnessSelfHealer {
    /// Bước 1 (Propose): Tạo prompt tinh chỉnh đối sách sau khi gặp lỗi trong quá trình thực thi Agent
    pub fn propose_agent_refinement(
        original_prompt: &str,
        error_msg: &str,
        attempt: u32,
        max_attempts: u32,
        lessons: &[HarnessLesson],
    ) -> String {
        let mut lesson_prompt = String::new();
        if !lessons.is_empty() {
            lesson_prompt
                .push_str("\n\n[Harness² Collective Memory - Bài học kinh nghiệm đã tích lũy]:\n");
            for (idx, les) in lessons.iter().enumerate() {
                lesson_prompt.push_str(&format!(
                    " - #{}: Lỗi trước đó '{}' -> Khắc phục: '{}'\n",
                    idx + 1,
                    les.error_pattern,
                    les.remediation_strategy
                ));
            }
        }

        format!(
            "{}\n\n======================================================\n\
            ⚠️ [Harness² Self-Healing Feedback - Thử Thách Lần {}/{}]:\n\
            Hệ thống phát hiện lượt thực thi trước đó chưa đạt chuẩn hoặc phát sinh sự cố:\n\
            >>> Chi tiết lỗi: {}\n\
            {}\n\
            Yêu cầu tự điều chỉnh (Self-Correction Directives):\n\
            1. Phân tích nguyên nhân gốc rễ dẫn đến sự cố trên.\n\
            2. Điều chỉnh trực tiếp nội dung output, tuân thủ nghiêm ngặt định dạng dữ liệu (JSON/Text thuần) theo yêu cầu.\n\
            3. Không lặp lại format lỗi và đảm bảo cung cấp kết quả hoàn chỉnh nhất.\n\
            ======================================================",
            original_prompt,
            attempt,
            max_attempts,
            error_msg,
            lesson_prompt
        )
    }

    /// Bước 1 (Propose) cho Tool Node: Đề xuất khắc phục lệnh shell hoặc cờ an toàn khi tool thất bại
    pub fn propose_tool_refinement(
        original_cmd: &str,
        stderr: &str,
        exit_code: i32,
    ) -> Option<String> {
        let stderr_lower = stderr.to_lowercase();

        // 1. Nếu thiếu thư mục tạo file output -> tự động chèn mkdir -p
        if stderr_lower.contains("no such file or directory") || stderr_lower.contains("not found")
        {
            if let Some(pos) = original_cmd.find("--output-dir ") {
                let rest = original_cmd.get(pos + 13..).unwrap_or("");
                let out_dir = rest.split_whitespace().next().unwrap_or("storage");
                return Some(format!("mkdir -p {} && {}", out_dir, original_cmd));
            }
        }

        // 2. Nếu thiếu permission -> thử chạy với bash trực tiếp hoặc sanitize path
        if exit_code == 127 || stderr_lower.contains("permission denied") {
            return Some(format!(
                "chmod +x scripts/*.py 2>/dev/null; {}",
                original_cmd
            ));
        }

        None
    }
}
