use std::collections::HashMap;
use std::path::Path;

use super::fs::copy_credential_dir;
use super::AccountVault;
use crate::accounts::types::{AccountEngineKind, AccountProfile};

/// Xây dựng bảng biến môi trường cách ly (Process Environment Injection) cho CLI từ AccountProfile
pub fn build_account_env(account: &AccountProfile) -> HashMap<String, String> {
    let mut envs = HashMap::new();
    let home_str = account.home_dir.to_string_lossy().to_string();

    match &account.engine {
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

impl AccountVault {
    /// Tự động sao chép credentials/session của CLI đang có sẵn trên máy host vào sandbox của tài khoản
    pub fn import_local_credentials(
        &self,
        engine: &AccountEngineKind,
        id: &str,
    ) -> Result<bool, std::io::Error> {
        if let AccountEngineKind::Generic(name) = engine {
            if matches!(
                name.trim().to_ascii_lowercase().as_str(),
                "antigravity" | "agy" | "gemini"
            ) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Importing local Google/Antigravity credentials is disabled to comply with Google Terms of Service",
                ));
            }
        }

        let account = self.get_account(engine, id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Account '{}' not found", id),
            )
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
            AccountEngineKind::Codex => {
                let candidates = [
                    (host_path.join(".codex"), account.home_dir.join(".codex")),
                    (
                        host_path.join(".config").join("codex"),
                        account.home_dir.join(".config").join("codex"),
                    ),
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
        build_account_env(account)
    }
}
