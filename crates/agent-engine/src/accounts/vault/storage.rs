use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::AccountVault;
use crate::accounts::types::{AccountEngineKind, AccountProfile, AccountStatus};

/// Đọc và deserialize danh sách AccountProfile từ file metadata JSON.
pub fn load_profiles_from_file(meta_file: &Path) -> Result<Vec<AccountProfile>, std::io::Error> {
    if !meta_file.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(meta_file)?;
    serde_json::from_str::<Vec<AccountProfile>>(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Serialize và ghi danh sách AccountProfile vào file metadata JSON an toàn.
pub fn save_profiles_to_file(
    base_dir: &Path,
    meta_file: &Path,
    profiles: &[AccountProfile],
) -> Result<(), std::io::Error> {
    if !base_dir.exists() {
        std::fs::create_dir_all(base_dir)?;
    }
    let json = serde_json::to_string_pretty(profiles)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(meta_file, json)
}

impl AccountVault {
    pub fn metadata_file(&self) -> PathBuf {
        self.base_dir.join("accounts.json")
    }

    pub(crate) fn load_sync(&self) {
        let meta_file = self.metadata_file();
        if meta_file.exists() {
            if let Ok(profiles) = load_profiles_from_file(&meta_file) {
                let mut lock = self.accounts_write();
                lock.clear();
                for profile in profiles {
                    lock.entry(profile.engine.clone())
                        .or_default()
                        .push(profile);
                }
            }
        }
    }

    pub fn save_sync(&self) -> Result<(), std::io::Error> {
        let lock = self.accounts_read();
        let mut all = Vec::new();
        for list in lock.values() {
            all.extend(list.clone());
        }
        save_profiles_to_file(&self.base_dir, &self.metadata_file(), &all)
    }

    /// Chọn tài khoản tiếp theo theo thuật toán xoay vòng Round-Robin & Kiểm tra Cooldown
    pub fn select_next_account(&self, engine: &AccountEngineKind) -> Option<AccountProfile> {
        let mut lock = self.accounts_write();
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
        let rr_lock = self.rr_read();
        let current_rr = match rr_lock.get(engine) {
            Some(a) => a.fetch_add(1, Ordering::SeqCst),
            None => {
                drop(rr_lock);
                let mut write_rr = self.rr_write();
                let atomic = write_rr
                    .entry(engine.clone())
                    .or_insert_with(|| AtomicUsize::new(0));
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
    pub fn mark_cooldown(
        &self,
        engine: &AccountEngineKind,
        id: &str,
        duration_secs: u64,
        reason: &str,
    ) {
        {
            let mut lock = self.accounts_write();
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
            let mut lock = self.accounts_write();
            if let Some(list) = lock.get_mut(engine) {
                if let Some(acc) = list.iter_mut().find(|a| a.id == id) {
                    acc.failure_count = 0;
                    acc.status = AccountStatus::Ready;
                }
            }
        }
        let _ = self.save_sync();
    }

    /// Đưa tài khoản ra khỏi trạng thái Cooldown về lại Ready ngay lập tức
    pub fn reset_cooldown(&self, engine: &AccountEngineKind, id: &str) -> bool {
        let updated = {
            let mut lock = self.accounts_write();
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
}
