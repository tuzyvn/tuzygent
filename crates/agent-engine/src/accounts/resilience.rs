mod cache;
mod selection;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::accounts::fault::AccountFaultPolicy;
use crate::accounts::types::AccountProfile;
use crate::accounts::vault::AccountVault;

#[derive(Clone)]
pub struct ResilientAccountManager {
    vault: Arc<AccountVault>,
    fault_policy: AccountFaultPolicy,
    max_retries: usize,
    resolved_count: usize,
    recovered_count: usize,
    rejected_count: usize,
    cooldown_count: usize,
    resolved_cache: HashMap<String, AccountProfile>,
    account_queue: VecDeque<String>,
    max_capacity: Option<usize>,
}

impl ResilientAccountManager {
    pub fn new(
        vault: Arc<AccountVault>,
        fault_policy: AccountFaultPolicy,
        max_retries: usize,
    ) -> Self {
        Self {
            vault,
            fault_policy,
            max_retries,
            resolved_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            cooldown_count: 0,
            resolved_cache: HashMap::new(),
            account_queue: VecDeque::new(),
            max_capacity: None,
        }
    }

    pub fn with_vault(vault: Arc<AccountVault>) -> Self {
        Self::new(vault, AccountFaultPolicy::none(), 3)
    }

    pub fn with_fault_policy(vault: Arc<AccountVault>, fault_policy: AccountFaultPolicy) -> Self {
        Self::new(vault, fault_policy, 3)
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.resolved_cache.reserve(capacity);
        self.account_queue.reserve(capacity);
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.evict_if_needed();
        self
    }
}
