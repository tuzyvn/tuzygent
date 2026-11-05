use std::collections::HashMap;

use crate::accounts::types::{AccountEngineKind, AccountProfile, AccountStatus};
use super::AccountVault;

/// Kiểm tra tính hợp lệ và chuẩn hóa ID tài khoản
pub fn sanitize_account_id(id: &str) -> Result<String, std::io::Error> {
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

    Ok(safe_id)
}

/// Kiểm tra tính an toàn của tên engine khi tạo đường dẫn sandbox
pub fn validate_engine_name(engine_str: &str) -> Result<(), std::io::Error> {
    if engine_str.contains('/') || engine_str.contains('\\') || engine_str.contains("..") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Invalid engine name '{}': path traversal detected", engine_str),
        ));
    }
    Ok(())
}

impl AccountVault {
    /// Đăng ký một tài khoản mới vào hệ thống với thư mục sandbox cách ly riêng
    pub fn register_account(
        &self,
        engine: AccountEngineKind,
        id: &str,
        name: &str,
        extra_env: HashMap<String, String>,
    ) -> Result<AccountProfile, std::io::Error> {
        let safe_id = sanitize_account_id(id)?;

        let engine_str = engine.to_string();
        validate_engine_name(&engine_str)?;

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
            let mut lock = self.accounts_write();
            let list = lock.entry(engine).or_default();
            list.retain(|a| a.id != profile.id);
            list.push(profile.clone());
        }

        self.save_sync()?;
        Ok(profile)
    }

    /// Lấy danh sách toàn bộ tài khoản của một engine hoặc tất cả engine
    pub fn list_accounts(&self, engine: Option<&AccountEngineKind>) -> Vec<AccountProfile> {
        let lock = self.accounts_read();
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
            let mut lock = self.accounts_write();
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

    /// Lấy thông tin một tài khoản cụ thể
    pub fn get_account(&self, engine: &AccountEngineKind, id: &str) -> Option<AccountProfile> {
        let lock = self.accounts_read();
        lock.get(engine)?.iter().find(|a| a.id == id).cloned()
    }
}
