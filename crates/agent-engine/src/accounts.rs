use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AccountEngineKind {
    Antigravity,
    Codex,
    Claude,
    Generic(String),
}

impl std::fmt::Display for AccountEngineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountEngineKind::Antigravity => write!(f, "antigravity"),
            AccountEngineKind::Codex => write!(f, "codex"),
            AccountEngineKind::Claude => write!(f, "claude"),
            AccountEngineKind::Generic(name) => write!(f, "{}", name),
        }
    }
}

impl AccountEngineKind {
    pub fn from_str_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "antigravity" | "agy" => AccountEngineKind::Antigravity,
            "codex" => AccountEngineKind::Codex,
            "claude" | "claude_code" | "claude-code" => AccountEngineKind::Claude,
            other => AccountEngineKind::Generic(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccountStatus {
    Ready,
    Cooldown {
        until: DateTime<Utc>,
        reason: String,
    },
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProfile {
    pub id: String,
    pub name: String,
    pub engine: AccountEngineKind,
    pub home_dir: PathBuf,
    pub status: AccountStatus,
    pub requests_count: u64,
    pub failure_count: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub extra_env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeAccountProfile {
    pub id: String,
    pub name: String,
    pub engine: AccountEngineKind,
    pub home_dir: PathBuf,
    pub status: AccountStatus,
    pub requests_count: u64,
    pub failure_count: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub extra_env_keys: Vec<String>,
}

impl AccountProfile {
    pub fn is_available(&self) -> bool {
        match &self.status {
            AccountStatus::Ready => true,
            AccountStatus::Cooldown { until, .. } => Utc::now() >= *until,
            AccountStatus::Disabled => false,
        }
    }

    pub fn to_safe(&self) -> SafeAccountProfile {
        SafeAccountProfile {
            id: self.id.clone(),
            name: self.name.clone(),
            engine: self.engine.clone(),
            home_dir: self.home_dir.clone(),
            status: self.status.clone(),
            requests_count: self.requests_count,
            failure_count: self.failure_count,
            last_used_at: self.last_used_at,
            extra_env_keys: self.extra_env.keys().cloned().collect(),
        }
    }
}

pub struct AccountVault {
    base_dir: PathBuf,
    accounts: Arc<RwLock<HashMap<AccountEngineKind, Vec<AccountProfile>>>>,
    round_robin_indices: Arc<RwLock<HashMap<AccountEngineKind, AtomicUsize>>>,
}

impl AccountVault {
    pub fn default_dir() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Path::new(&home).join(".agentcore").join("accounts")
    }

    pub fn new() -> Self {
        Self::new_with_dir(Self::default_dir())
    }

    pub fn new_with_dir(base_dir: PathBuf) -> Self {
        let vault = Self {
            base_dir,
            accounts: Arc::new(RwLock::new(HashMap::new())),
            round_robin_indices: Arc::new(RwLock::new(HashMap::new())),
        };
        vault.load_sync();
        vault
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn metadata_file(&self) -> PathBuf {
        self.base_dir.join("accounts.json")
    }

    fn load_sync(&self) {
        let meta_file = self.metadata_file();
        if meta_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&meta_file) {
                if let Ok(profiles) = serde_json::from_str::<Vec<AccountProfile>>(&content) {
                    let mut lock = self.accounts.write().unwrap();
                    lock.clear();
                    for profile in profiles {
                        lock.entry(profile.engine.clone())
                            .or_default()
                            .push(profile);
                    }
                }
            }
        }
    }

    pub fn save_sync(&self) -> Result<(), std::io::Error> {
        if !self.base_dir.exists() {
            std::fs::create_dir_all(&self.base_dir)?;
        }
        let lock = self.accounts.read().unwrap();
        let mut all = Vec::new();
        for list in lock.values() {
            all.extend(list.clone());
        }
        let json = serde_json::to_string_pretty(&all)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(self.metadata_file(), json)
    }

    /// Đăng ký một tài khoản mới vào hệ thống với thư mục sandbox cách ly riêng
    pub fn register_account(
        &self,
        engine: AccountEngineKind,
        id: &str,
        name: &str,
        extra_env: HashMap<String, String>,
    ) -> Result<AccountProfile, std::io::Error> {
        let trimmed_id = id.trim();
        if trimmed_id.is_empty()
            || trimmed_id == "."
            || trimmed_id == ".."
            || trimmed_id.contains('/')
            || trimmed_id.contains('\\')
            || trimmed_id.contains("..")
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid account ID '{}': must not be empty or contain path traversal characters", id),
            ));
        }

        let safe_id = trimmed_id.to_lowercase().replace(' ', "_");
        if !safe_id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid account ID '{}': only alphanumeric, underscore, and dash are allowed", id),
            ));
        }

        let engine_str = engine.to_string();
        if engine_str.contains('/') || engine_str.contains('\\') || engine_str.contains("..") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid engine name '{}': path traversal detected", engine_str),
            ));
        }

        let account_home = self.base_dir.join(&engine_str).join(&safe_id);

        if !account_home.exists() {
            std::fs::create_dir_all(&account_home)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&account_home, std::fs::Permissions::from_mode(0o700));
            }
        }

        let profile = AccountProfile {
            id: safe_id,
            name: name.to_string(),
            engine: engine.clone(),
            home_dir: account_home,
            status: AccountStatus::Ready,
            requests_count: 0,
            failure_count: 0,
            last_used_at: None,
            extra_env,
        };

        {
            let mut lock = self.accounts.write().unwrap();
            let list = lock.entry(engine).or_default();
            list.retain(|a| a.id != profile.id);
            list.push(profile.clone());
        }

        self.save_sync()?;
        Ok(profile)
    }

    /// Lấy danh sách toàn bộ tài khoản của một engine hoặc tất cả engine
    pub fn list_accounts(&self, engine: Option<&AccountEngineKind>) -> Vec<AccountProfile> {
        let lock = self.accounts.read().unwrap();
        if let Some(eng) = engine {
            lock.get(eng).cloned().unwrap_or_default()
        } else {
            let mut all = Vec::new();
            for list in lock.values() {
                all.extend(list.clone());
            }
            all
        }
    }

    /// Xóa một tài khoản khỏi pool
    pub fn remove_account(&self, engine: &AccountEngineKind, id: &str) -> Result<bool, std::io::Error> {
        let removed = {
            let mut lock = self.accounts.write().unwrap();
            if let Some(list) = lock.get_mut(engine) {
                let initial_len = list.len();
                list.retain(|a| a.id != id);
                list.len() < initial_len
            } else {
                false
            }
        };

        if removed {
            self.save_sync()?;
        }
        Ok(removed)
    }

    /// Chọn tài khoản tiếp theo theo thuật toán xoay vòng Round-Robin & Kiểm tra Cooldown
    pub fn select_next_account(&self, engine: &AccountEngineKind) -> Option<AccountProfile> {
        let mut lock = self.accounts.write().unwrap();
        let list = lock.get_mut(engine)?;
        if list.is_empty() {
            return None;
        }

        // Tự động giải phóng Cooldown nếu đã hết hạn
        let now = Utc::now();
        for acc in list.iter_mut() {
            if let AccountStatus::Cooldown { until, .. } = &acc.status {
                if now >= *until {
                    acc.status = AccountStatus::Ready;
                }
            }
        }

        // Lọc danh sách các tài khoản đang sẵn sàng
        let ready_indices: Vec<usize> = list
            .iter()
            .enumerate()
            .filter(|(_, a)| a.is_available())
            .map(|(i, _)| i)
            .collect();

        if ready_indices.is_empty() {
            return None;
        }

        // Lấy con trỏ xoay vòng
        let rr_lock = self.round_robin_indices.read().unwrap();
        let current_rr = match rr_lock.get(engine) {
            Some(a) => a.fetch_add(1, Ordering::SeqCst),
            None => {
                drop(rr_lock);
                let mut write_rr = self.round_robin_indices.write().unwrap();
                let atomic = write_rr.entry(engine.clone()).or_insert_with(|| AtomicUsize::new(0));
                atomic.fetch_add(1, Ordering::SeqCst)
            }
        };

        let chosen_idx = ready_indices[current_rr % ready_indices.len()];

        let chosen = &mut list[chosen_idx];
        chosen.requests_count += 1;
        chosen.last_used_at = Some(now);
        Some(chosen.clone())
    }

    /// Đánh dấu tài khoản vào trạng thái Cooldown khi gặp Rate Limit 429 hoặc lỗi quota
    pub fn mark_cooldown(&self, engine: &AccountEngineKind, id: &str, duration_secs: u64, reason: &str) {
        {
            let mut lock = self.accounts.write().unwrap();
            if let Some(list) = lock.get_mut(engine) {
                if let Some(acc) = list.iter_mut().find(|a| a.id == id) {
                    acc.failure_count += 1;
                    acc.status = AccountStatus::Cooldown {
                        until: Utc::now() + chrono::Duration::seconds(duration_secs as i64),
                        reason: reason.to_string(),
                    };
                }
            }
        }
        let _ = self.save_sync();
    }

    /// Ghi nhận thực thi thành công
    pub fn record_success(&self, engine: &AccountEngineKind, id: &str) {
        {
            let mut lock = self.accounts.write().unwrap();
            if let Some(list) = lock.get_mut(engine) {
                if let Some(acc) = list.iter_mut().find(|a| a.id == id) {
                    acc.failure_count = 0;
                    acc.status = AccountStatus::Ready;
                }
            }
        }
        let _ = self.save_sync();
    }

    /// Lấy thông tin một tài khoản cụ thể
    pub fn get_account(&self, engine: &AccountEngineKind, id: &str) -> Option<AccountProfile> {
        let lock = self.accounts.read().unwrap();
        lock.get(engine)?.iter().find(|a| a.id == id).cloned()
    }

    /// Đưa tài khoản ra khỏi trạng thái Cooldown về lại Ready ngay lập tức
    pub fn reset_cooldown(&self, engine: &AccountEngineKind, id: &str) -> bool {
        let updated = {
            let mut lock = self.accounts.write().unwrap();
            if let Some(list) = lock.get_mut(engine) {
                if let Some(acc) = list.iter_mut().find(|a| a.id == id) {
                    acc.status = AccountStatus::Ready;
                    acc.failure_count = 0;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if updated {
            let _ = self.save_sync();
        }
        updated
    }

    /// Tự động sao chép credentials/session của CLI đang có sẵn trên máy host vào sandbox của tài khoản
    pub fn import_local_credentials(&self, engine: &AccountEngineKind, id: &str) -> Result<bool, std::io::Error> {
        let account = self.get_account(engine, id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("Account '{}' not found", id))
        })?;

        let host_home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        if host_home.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME environment variable not set",
            ));
        }
        let host_path = Path::new(&host_home);
        let mut imported = false;

        match engine {
            AccountEngineKind::Antigravity => {
                let candidates = [
                    (host_path.join(".gemini"), account.home_dir.join(".gemini")),
                    (host_path.join(".config").join("antigravity"), account.home_dir.join(".config").join("antigravity")),
                ];
                for (src, dst) in &candidates {
                    if src.exists() && src.is_dir() {
                        copy_credential_dir(src, dst)?;
                        imported = true;
                    }
                }
            }
            AccountEngineKind::Codex => {
                let candidates = [
                    (host_path.join(".codex"), account.home_dir.join(".codex")),
                    (host_path.join(".config").join("codex"), account.home_dir.join(".config").join("codex")),
                ];
                for (src, dst) in &candidates {
                    if src.exists() && src.is_dir() {
                        copy_credential_dir(src, dst)?;
                        imported = true;
                    }
                }
            }
            AccountEngineKind::Claude => {
                let src = host_path.join(".claude");
                let dst = account.home_dir.join(".claude");
                if src.exists() && src.is_dir() {
                    copy_credential_dir(&src, &dst)?;
                    imported = true;
                }
            }
            _ => {}
        }

        if !imported {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No local credentials found on host for engine '{}'", engine),
            ));
        }
        Ok(true)
    }

    /// Xây dựng bảng biến môi trường cách ly (Process Environment Injection) cho CLI
    pub fn build_env_for_account(&self, account: &AccountProfile) -> HashMap<String, String> {
        let mut envs = HashMap::new();
        let home_str = account.home_dir.to_string_lossy().to_string();

        match &account.engine {
            AccountEngineKind::Antigravity => {
                // Ép agy đọc config & token từ thư mục cách ly của tài khoản này
                envs.insert("HOME".to_string(), home_str.clone());
                envs.insert("GEMINI_CLI_HOME".to_string(), home_str.clone());
                envs.insert("ANTIGRAVITY_CONFIG_DIR".to_string(), home_str.clone());
                envs.insert("ANTIGRAVITY_HOME".to_string(), home_str);
            }
            AccountEngineKind::Codex => {
                // Ép codex đọc config & token từ thư mục cách ly của tài khoản này
                envs.insert("HOME".to_string(), home_str.clone());
                envs.insert("CODEX_HOME".to_string(), home_str.clone());
                envs.insert("CODEX_CONFIG_DIR".to_string(), home_str);
            }
            AccountEngineKind::Claude => {
                // Ép claude code đọc config từ thư mục cách ly
                envs.insert("HOME".to_string(), home_str.clone());
                envs.insert("CLAUDE_CONFIG_DIR".to_string(), home_str);
            }
            AccountEngineKind::Generic(_) => {
                envs.insert("HOME".to_string(), home_str);
            }
        }

        // Bổ sung các biến môi trường cấu hình thêm (ví dụ OPENAI_API_KEY nếu có)
        for (k, v) in &account.extra_env {
            envs.insert(k.clone(), v.clone());
        }

        envs
    }
}

fn copy_credential_dir(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o700));
        }
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_credential_dir(&path, &target)?;
        } else {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let is_credential = file_name.ends_with(".json")
                || file_name.ends_with(".toml")
                || file_name.ends_with(".yaml")
                || file_name.ends_with(".yml")
                || file_name.contains("token")
                || file_name.contains("auth")
                || file_name.contains("cred")
                || file_name.contains("session")
                || file_name.contains("config");
            if is_credential {
                std::fs::copy(&path, &target)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    copy_credential_dir(src, dst)
}

impl Default for AccountVault {
    fn default() -> Self {
        Self::new()
    }
}

/// ComputeEngine tự động xoay vòng tài khoản CLI (Antigravity, Codex...)
/// theo danh sách tài khoản trong AccountVault
pub struct AccountRotatedEngine {
    vault: Arc<AccountVault>,
    engine_kind: AccountEngineKind,
    binary_path: Option<String>,
    model: Option<String>,
}

impl AccountRotatedEngine {
    pub fn new(
        vault: Arc<AccountVault>,
        engine_kind: AccountEngineKind,
        binary_path: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            vault,
            engine_kind,
            binary_path,
            model,
        }
    }
}

#[async_trait]
impl ComputeEngine for AccountRotatedEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let max_retries = 3;
        let mut last_err = None;

        let accounts = self.vault.list_accounts(Some(&self.engine_kind));
        if accounts.is_empty() {
            return Err(EngineError::ProcessFailed(format!(
                "No account configured for engine '{}'. Please register an account first.",
                self.engine_kind
            )));
        }

        for _ in 0..max_retries {
            let account = match self.vault.select_next_account(&self.engine_kind) {
                Some(acc) => acc,
                None => {
                    return Err(EngineError::ProcessFailed(format!(
                        "All accounts for engine '{}' are currently in cooldown or unavailable.",
                        self.engine_kind
                    )));
                }
            };
            let env = self.vault.build_env_for_account(&account);

            let engine: Arc<dyn ComputeEngine> = match &self.engine_kind {
                AccountEngineKind::Antigravity => {
                    Arc::new(crate::cli::AntigravityEngine::new(self.binary_path.clone(), self.model.clone()).with_env(env))
                }
                AccountEngineKind::Codex => {
                    Arc::new(crate::cli::CodexEngine::new(self.binary_path.clone(), self.model.clone()).with_env(env))
                }
                AccountEngineKind::Claude => {
                    Arc::new(crate::cli::ClaudeCodeEngine::new(self.binary_path.clone(), self.model.clone()).with_env(env))
                }
                AccountEngineKind::Generic(name) => {
                    if name.contains('/') || name.contains('\\') || name.contains("..") {
                        return Err(EngineError::ProcessFailed(format!("Invalid generic engine name '{}'", name)));
                    }
                    Arc::new(crate::cli::GenericCliEngine::new(
                        name.clone(),
                        self.binary_path.clone().unwrap_or_else(|| name.clone()),
                        vec![],
                        env,
                        None,
                    ))
                }
            };

            match engine.execute(input).await {
                Ok(output) => {
                    self.vault.record_success(&self.engine_kind, &account.id);
                    return Ok(output);
                }
                Err(err) => {
                    let cooldown_secs = if err.is_auth_or_quota() { 300 } else { 60 };
                    self.vault.mark_cooldown(&self.engine_kind, &account.id, cooldown_secs, &err.to_string());
                    last_err = Some(err);
                    if self.vault.list_accounts(Some(&self.engine_kind)).len() <= 1 {
                        break;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| EngineError::ProcessFailed("Account pool exhausted or execution failed".to_string())))
    }

    fn engine_kind(&self) -> EngineKind {
        match &self.engine_kind {
            AccountEngineKind::Antigravity => EngineKind::Antigravity,
            AccountEngineKind::Codex => EngineKind::Codex,
            AccountEngineKind::Claude => EngineKind::ClaudeCode,
            AccountEngineKind::Generic(n) => EngineKind::Generic(n.clone()),
        }
    }
}
