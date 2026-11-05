use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncWriteExt;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DurableMemoryError {
    #[error("Socket timeout communicating with durable storage")]
    SocketTimeout,
    #[error("Connection reset by peer during memory operation")]
    ConnectionReset,
    #[error("Transient I/O failure during durable memory operation: {0}")]
    TransientIo(String),
    #[error("Corrupted durable state or malformed memory structure: {0}")]
    CorruptedState(String),
    #[error("Lock acquisition failed or file lock contention: {0}")]
    LockFailure(String),
    #[error("Content cannot be empty or blank")]
    EmptyContent,
    #[error("Max retry attempts ({0}) exceeded during durable memory operation")]
    MaxRetriesExceeded(usize),
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<std::io::Error> for DurableMemoryError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::TimedOut => DurableMemoryError::SocketTimeout,
            std::io::ErrorKind::ConnectionReset => DurableMemoryError::ConnectionReset,
            std::io::ErrorKind::InvalidData => DurableMemoryError::CorruptedState(err.to_string()),
            _ => DurableMemoryError::Io(err.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurableMemoryFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    CorruptedState,
    LockFailure,
}

impl DurableMemoryFault {
    pub fn to_error(&self) -> DurableMemoryError {
        match self {
            Self::SocketTimeout => DurableMemoryError::SocketTimeout,
            Self::ConnectionReset => DurableMemoryError::ConnectionReset,
            Self::TransientIo => DurableMemoryError::TransientIo(
                "Simulated transient I/O failure during durable memory operation".to_string(),
            ),
            Self::CorruptedState => DurableMemoryError::CorruptedState(
                "Simulated corrupted or unparseable durable memory structure".to_string(),
            ),
            Self::LockFailure => DurableMemoryError::LockFailure(
                "Simulated lock acquisition failure due to contention".to_string(),
            ),
        }
    }

    pub fn simulate_fault(&self) -> Result<(), DurableMemoryError> {
        Err(self.to_error())
    }
}

pub fn simulate_durable_fault(fault: &DurableMemoryFault) -> Result<(), DurableMemoryError> {
    fault.simulate_fault()
}

#[derive(Debug, Clone, Default)]
pub struct DurableMemoryFaultPolicy {
    fault: Option<DurableMemoryFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl DurableMemoryFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: DurableMemoryFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: DurableMemoryFault, interval: u32) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: Some(interval.max(1)),
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn with_max_injections(mut self, max: usize) -> Self {
        self.max_injections = Some(max);
        self
    }

    pub fn should_inject(&mut self) -> Option<DurableMemoryFault> {
        let fault = self.fault.as_ref()?;

        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);

        let inject = match self.fail_interval {
            Some(interval) if interval > 0 => (self.invocation_count - 1).is_multiple_of(interval),
            Some(_) => true,
            None => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }
}

/// Serialize read-modify-write operations across processes sharing a workspace.
/// The lock file must remain in place so all writers lock the same inode.
pub async fn lock_memory_file(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent).await;
    }
    let lock_path = path.with_extension("lock");
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(lock_path)?;
        file.lock()?;
        Ok(file)
    }).await.map_err(std::io::Error::other)?
}

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

#[derive(Debug, Clone)]
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

    pub fn memory_file(&self) -> &Path {
        &self.memory_file
    }

    pub fn user_file(&self) -> &Path {
        &self.user_file
    }

    pub fn lessons_json_file(&self) -> &Path {
        &self.lessons_json_file
    }

    pub fn legacy_lessons_file(&self) -> &Path {
        &self.legacy_lessons_file
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
        let _lock = lock_memory_file(&self.lessons_json_file).await?;
        let mut lessons = self.load_structured_lessons().await?;
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

/// Resilient wrapper around `DurableMemoryStore` that injects fault policies,
/// recovers from transient network/IO errors with retry loops, protects against state corruption,
/// and records operational audits into `AuditLedger`.
pub type ResilientDurableMemory = ResilientDurableMemoryStore;

#[derive(Clone)]
pub struct ResilientDurableMemoryStore {
    store: DurableMemoryStore,
    fault_policy: DurableMemoryFaultPolicy,
    max_retries: usize,
    saved_count: usize,
    loaded_count: usize,
    recovered_count: usize,
    rejected_count: usize,
}

impl ResilientDurableMemoryStore {
    pub fn new(
        store: DurableMemoryStore,
        fault_policy: DurableMemoryFaultPolicy,
        max_retries: usize,
    ) -> Self {
        Self {
            store,
            fault_policy,
            max_retries,
            saved_count: 0,
            loaded_count: 0,
            recovered_count: 0,
            rejected_count: 0,
        }
    }

    pub fn with_store(store: DurableMemoryStore) -> Self {
        Self::new(store, DurableMemoryFaultPolicy::none(), 3)
    }

    pub fn with_policy(store: DurableMemoryStore, fault_policy: DurableMemoryFaultPolicy) -> Self {
        Self::new(store, fault_policy, 3)
    }

    pub fn open<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self::with_store(DurableMemoryStore::new(workspace_dir))
    }

    pub fn from_workspace<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self::with_store(DurableMemoryStore::new(workspace_dir))
    }

    pub fn with_fault_policy(mut self, fault_policy: DurableMemoryFaultPolicy) -> Self {
        self.fault_policy = fault_policy;
        self
    }

    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn store(&self) -> &DurableMemoryStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut DurableMemoryStore {
        &mut self.store
    }

    pub fn fault_policy(&self) -> &DurableMemoryFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut DurableMemoryFaultPolicy {
        &mut self.fault_policy
    }

    pub fn policy(&self) -> &DurableMemoryFaultPolicy {
        &self.fault_policy
    }

    pub fn policy_mut(&mut self) -> &mut DurableMemoryFaultPolicy {
        &mut self.fault_policy
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn set_policy(&mut self, policy: DurableMemoryFaultPolicy) {
        self.fault_policy = policy;
    }

    pub fn set_max_retries(&mut self, max_retries: usize) {
        self.max_retries = max_retries;
    }

    pub fn saved_count(&self) -> usize {
        self.saved_count
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn reset_counts(&mut self) {
        self.saved_count = 0;
        self.loaded_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
    }

    fn check_fault_injection(&mut self, attempt: usize) -> Result<Option<()>, DurableMemoryError> {
        if let Some(fault) = self.fault_policy.should_inject() {
            match fault {
                DurableMemoryFault::SocketTimeout
                | DurableMemoryFault::ConnectionReset
                | DurableMemoryFault::TransientIo
                | DurableMemoryFault::LockFailure => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        Ok(Some(()))
                    } else {
                        Err(fault.to_error())
                    }
                }
                DurableMemoryFault::CorruptedState => {
                    self.rejected_count += 1;
                    Err(fault.to_error())
                }
            }
        } else {
            Ok(None)
        }
    }

    fn handle_transient_error(
        &mut self,
        attempt: usize,
        err: std::io::Error,
    ) -> Result<(), DurableMemoryError> {
        if attempt <= self.max_retries {
            self.recovered_count += 1;
            Ok(())
        } else {
            Err(DurableMemoryError::from(err))
        }
    }

    pub async fn save_lesson(&mut self, lesson: &str) -> Result<(), DurableMemoryError> {
        if lesson.trim().is_empty() {
            self.rejected_count += 1;
            return Err(DurableMemoryError::EmptyContent);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.save_lesson(lesson).await {
                Ok(()) => {
                    self.saved_count += 1;
                    return Ok(());
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn save_structured_lesson(
        &mut self,
        lesson: &StructuredLesson,
    ) -> Result<(), DurableMemoryError> {
        if lesson.id.trim().is_empty() || lesson.task_title.trim().is_empty() {
            self.rejected_count += 1;
            return Err(DurableMemoryError::EmptyContent);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.save_structured_lesson(lesson).await {
                Ok(()) => {
                    self.saved_count += 1;
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    // Auto-recover from corrupt json file:
                    // Backup already made by load_structured_lessons.
                    // Reset the corrupted file and retry once to prevent permanent write failure.
                    let _ = fs::remove_file(&self.store.lessons_json_file).await;
                    self.recovered_count += 1;
                    match self.store.save_structured_lesson(lesson).await {
                        Ok(()) => {
                            self.saved_count += 1;
                            return Ok(());
                        }
                        Err(e2) => {
                            self.rejected_count += 1;
                            return Err(DurableMemoryError::from(e2));
                        }
                    }
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn load_relevant_context(&mut self, goal: &str) -> Result<String, DurableMemoryError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.load_relevant_context(goal).await {
                Ok(ctx) => {
                    self.loaded_count += 1;
                    return Ok(ctx);
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn load_structured_lessons(
        &mut self,
    ) -> Result<Vec<StructuredLesson>, DurableMemoryError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.load_structured_lessons().await {
                Ok(lessons) => {
                    self.loaded_count += 1;
                    return Ok(lessons);
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn save_user_directive(&mut self, directive: &str) -> Result<(), DurableMemoryError> {
        if directive.trim().is_empty() {
            self.rejected_count += 1;
            return Err(DurableMemoryError::EmptyContent);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.save_user_directive(directive).await {
                Ok(()) => {
                    self.saved_count += 1;
                    return Ok(());
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn save_lesson_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        lesson: &str,
    ) -> Result<(), DurableMemoryError> {
        if lesson.trim().is_empty() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                "Durable store rejected empty lesson entry".to_string(),
            );
            return Err(DurableMemoryError::EmptyContent);
        }

        match self.save_lesson(lesson).await {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Durable lesson successfully saved: '{}'", lesson.trim()),
                );
                Ok(())
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable save_lesson failure: {}", err),
                );
                Err(err)
            }
        }
    }

    pub async fn save_structured_lesson_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        lesson: &StructuredLesson,
    ) -> Result<(), DurableMemoryError> {
        if lesson.id.trim().is_empty() || lesson.task_title.trim().is_empty() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                "Durable store rejected invalid structured lesson".to_string(),
            );
            return Err(DurableMemoryError::EmptyContent);
        }

        match self.save_structured_lesson(lesson).await {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Structured lesson successfully saved: '{}'", lesson.id),
                );
                Ok(())
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable save_structured_lesson failure: {}", err),
                );
                Err(err)
            }
        }
    }

    pub async fn load_relevant_context_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        goal: &str,
    ) -> Result<String, DurableMemoryError> {
        match self.load_relevant_context(goal).await {
            Ok(ctx) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Relevant durable context loaded for goal: '{}'", goal),
                );
                Ok(ctx)
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable load_relevant_context failure: {}", err),
                );
                Err(err)
            }
        }
    }

    pub async fn save_user_directive_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        directive: &str,
    ) -> Result<(), DurableMemoryError> {
        if directive.trim().is_empty() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                "Durable store rejected empty user directive".to_string(),
            );
            return Err(DurableMemoryError::EmptyContent);
        }

        match self.save_user_directive(directive).await {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("User directive successfully saved: '{}'", directive.trim()),
                );
                Ok(())
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable save_user_directive failure: {}", err),
                );
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resilient_durable_memory_alias_and_builders() {
        let temp_dir = std::env::temp_dir().join(format!("test_rdm_{}", uuid::Uuid::new_v4().simple()));
        let mut mem: ResilientDurableMemory = ResilientDurableMemory::from_workspace(&temp_dir)
            .with_max_retries(5)
            .with_fault_policy(DurableMemoryFaultPolicy::intermittent(
                DurableMemoryFault::TransientIo,
                2,
            ));

        assert_eq!(mem.max_retries(), 5);
        assert_eq!(mem.saved_count(), 0);

        let res = mem.save_lesson("Architecture resilience verified").await;
        assert!(res.is_ok());
        assert_eq!(mem.saved_count(), 1);
        assert!(mem.recovered_count() > 0);

        let loaded = mem.load_relevant_context("resilience").await.unwrap();
        assert!(loaded.contains("Architecture resilience verified"));

        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_resilient_durable_memory_open_constructor() {
        let temp_dir = std::env::temp_dir().join(format!("test_rdm_open_{}", uuid::Uuid::new_v4().simple()));
        let mut mem = ResilientDurableMemory::open(&temp_dir);

        let lesson = StructuredLesson {
            id: "les-open-01".to_string(),
            timestamp: "2026-11-06T00:00:00Z".to_string(),
            task_title: "Open constructor check".to_string(),
            trigger_error: None,
            remediation_applied: None,
            general_insight: "Resilient memory constructor works seamlessly".to_string(),
            success: true,
        };

        assert!(mem.save_structured_lesson(&lesson).await.is_ok());
        let lessons = mem.load_structured_lessons().await.unwrap();
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].id, "les-open-01");

        let _ = fs::remove_dir_all(&temp_dir).await;
    }
}
