use super::ResilientAccountManager;
use crate::accounts::fault::AccountFaultPolicy;
use crate::accounts::types::AccountProfile;
use crate::accounts::vault::AccountVault;
use std::sync::Arc;

impl ResilientAccountManager {
    pub fn resolved_count(&self) -> usize {
        self.resolved_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn cooldown_count(&self) -> usize {
        self.cooldown_count
    }

    pub fn fault_policy(&self) -> &AccountFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut AccountFaultPolicy {
        &mut self.fault_policy
    }

    pub fn vault(&self) -> &Arc<AccountVault> {
        &self.vault
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn reset_counts(&mut self) {
        self.resolved_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
        self.cooldown_count = 0;
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let cache_overhead = self.resolved_cache.capacity()
            * (std::mem::size_of::<String>() + std::mem::size_of::<AccountProfile>());
        let queue_overhead = self.account_queue.capacity() * std::mem::size_of::<String>();
        let heap_content: usize = self
            .resolved_cache
            .values()
            .map(|profile| profile.estimated_size_bytes())
            .sum();
        base_size + cache_overhead + queue_overhead + heap_content
    }

    pub(super) fn evict_if_needed(&mut self) {
        if let Some(max) = self.max_capacity {
            while self.account_queue.len() >= max && max > 0 {
                if let Some(old_id) = self.account_queue.pop_front() {
                    self.resolved_cache.remove(&old_id);
                }
            }
        }
    }

    pub(super) fn cache_account(&mut self, profile: &AccountProfile) {
        if let Some(max) = self.max_capacity {
            if max == 0 {
                return;
            }
            while self.account_queue.len() >= max {
                if let Some(old_id) = self.account_queue.pop_front() {
                    self.resolved_cache.remove(&old_id);
                }
            }
            self.account_queue.push_back(profile.id.clone());
            self.resolved_cache
                .insert(profile.id.clone(), profile.clone());
        }
    }
}
