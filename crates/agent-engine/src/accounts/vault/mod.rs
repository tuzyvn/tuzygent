use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::types::{AccountEngineKind, AccountProfile};

pub mod env;
pub mod fs;
pub mod ops;
pub mod storage;

pub use env::*;
pub use fs::*;
pub use ops::*;
pub use storage::*;

pub struct AccountVault {
    pub(crate) base_dir: PathBuf,
    pub(crate) accounts: Arc<RwLock<HashMap<AccountEngineKind, Vec<AccountProfile>>>>,
    pub(crate) round_robin_indices: Arc<RwLock<HashMap<AccountEngineKind, AtomicUsize>>>,
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

    pub(crate) fn accounts_read(&self) -> RwLockReadGuard<'_, HashMap<AccountEngineKind, Vec<AccountProfile>>> {
        self.accounts.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn accounts_write(&self) -> RwLockWriteGuard<'_, HashMap<AccountEngineKind, Vec<AccountProfile>>> {
        self.accounts.write().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn rr_read(&self) -> RwLockReadGuard<'_, HashMap<AccountEngineKind, AtomicUsize>> {
        self.round_robin_indices.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn rr_write(&self) -> RwLockWriteGuard<'_, HashMap<AccountEngineKind, AtomicUsize>> {
        self.round_robin_indices.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

impl Default for AccountVault {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
