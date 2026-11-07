use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::accounts::fault::{AccountError, AccountFault, AccountFaultPolicy};
use crate::accounts::types::{AccountEngineKind, AccountProfile};
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
            .map(|p| p.estimated_size_bytes())
            .sum();
        base_size + cache_overhead + queue_overhead + heap_content
    }

    fn evict_if_needed(&mut self) {
        if let Some(max) = self.max_capacity {
            while self.account_queue.len() >= max && max > 0 {
                if let Some(old_id) = self.account_queue.pop_front() {
                    self.resolved_cache.remove(&old_id);
                }
            }
        }
    }

    fn cache_account(&mut self, profile: &AccountProfile) {
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

    pub fn select_account(
        &mut self,
        engine: &AccountEngineKind,
    ) -> Result<AccountProfile, AccountError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    AccountFault::RateLimitExceeded => {
                        if let Some(acc) = self.vault.select_next_account(engine) {
                            self.vault.mark_cooldown(
                                engine,
                                &acc.id,
                                60,
                                "RateLimitExceeded (Chaos Injected)",
                            );
                            self.cooldown_count += 1;
                            self.recovered_count += 1;
                            if attempt <= self.max_retries {
                                continue;
                            }
                            return Err(AccountError::RateLimitExceeded(
                                "Rate limit exceeded across account pool".to_string(),
                            ));
                        }
                        self.rejected_count += 1;
                        return Err(AccountError::NoAvailableAccount(format!(
                            "All accounts for engine '{}' are exhausted or in cooldown",
                            engine
                        )));
                    }
                    AccountFault::SocketTimeout
                    | AccountFault::ConnectionReset
                    | AccountFault::TransientIo
                    | AccountFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        }
                        return match fault {
                            AccountFault::SocketTimeout => Err(AccountError::SocketTimeout),
                            AccountFault::ConnectionReset => Err(AccountError::ConnectionReset),
                            AccountFault::TransientIo => Err(AccountError::TransientIo(
                                "Simulated transient I/O failure".to_string(),
                            )),
                            AccountFault::ServiceUnavailable => {
                                Err(AccountError::ServiceUnavailable(
                                    "Account service temporarily unavailable".to_string(),
                                ))
                            }
                            _ => unreachable!(),
                        };
                    }
                    AccountFault::CorruptedMetadata => {
                        return Err(AccountError::CorruptedMetadata(
                            "Account profile metadata corruption detected".to_string(),
                        ));
                    }
                }
            } else {
                match self.vault.select_next_account(engine) {
                    Some(profile) => match profile.validate() {
                        Ok(()) => {
                            self.cache_account(&profile);
                            self.resolved_count += 1;
                            return Ok(profile);
                        }
                        Err(err) => {
                            self.rejected_count += 1;
                            return Err(err);
                        }
                    },
                    None => {
                        self.rejected_count += 1;
                        return Err(AccountError::NoAvailableAccount(format!(
                            "No available accounts for engine '{}'",
                            engine
                        )));
                    }
                }
            }
        }
    }

    pub fn select_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        engine: &AccountEngineKind,
    ) -> Result<AccountProfile, AccountError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    AccountFault::RateLimitExceeded => {
                        if let Some(acc) = self.vault.select_next_account(engine) {
                            self.vault.mark_cooldown(
                                engine,
                                &acc.id,
                                60,
                                "RateLimitExceeded (Chaos Injected)",
                            );
                            self.cooldown_count += 1;
                            self.recovered_count += 1;
                            ledger.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!(
                                    "Account '{}' rate limited; cooldown triggered and rotated",
                                    acc.id
                                ),
                            );
                            if attempt <= self.max_retries {
                                continue;
                            }
                            let err = AccountError::RateLimitExceeded(
                                "Rate limit exceeded across account pool".to_string(),
                            );
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "Account selection exception for engine '{}': {}",
                                    engine, err
                                ),
                            );
                            return Err(err);
                        }
                        self.rejected_count += 1;
                        let err = AccountError::NoAvailableAccount(format!(
                            "All accounts for engine '{}' are exhausted or in cooldown",
                            engine
                        ));
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Account selection exception for engine '{}': {}",
                                engine, err
                            ),
                        );
                        return Err(err);
                    }
                    AccountFault::SocketTimeout
                    | AccountFault::ConnectionReset
                    | AccountFault::TransientIo
                    | AccountFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        }
                        let err = match fault {
                            AccountFault::SocketTimeout => AccountError::SocketTimeout,
                            AccountFault::ConnectionReset => AccountError::ConnectionReset,
                            AccountFault::TransientIo => AccountError::TransientIo(
                                "Simulated transient I/O failure".to_string(),
                            ),
                            AccountFault::ServiceUnavailable => AccountError::ServiceUnavailable(
                                "Account service temporarily unavailable".to_string(),
                            ),
                            _ => unreachable!(),
                        };
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Account selection exception for engine '{}': {}",
                                engine, err
                            ),
                        );
                        return Err(err);
                    }
                    AccountFault::CorruptedMetadata => {
                        let err = AccountError::CorruptedMetadata(
                            "Account profile metadata corruption detected".to_string(),
                        );
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Account selection exception for engine '{}': {}",
                                engine, err
                            ),
                        );
                        return Err(err);
                    }
                }
            } else {
                match self.vault.select_next_account(engine) {
                    Some(profile) => match profile.validate() {
                        Ok(()) => {
                            self.cache_account(&profile);
                            self.resolved_count += 1;
                            ledger.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!(
                                    "Account '{}' resolved for engine '{}'",
                                    profile.id, engine
                                ),
                            );
                            return Ok(profile);
                        }
                        Err(err) => {
                            self.rejected_count += 1;
                            ledger.record_event(
                                AuditEventType::CommandBlocked,
                                ActionSeverity::Destructive,
                                format!(
                                    "Account profile rejected for engine '{}': {}",
                                    engine, err
                                ),
                            );
                            return Err(err);
                        }
                    },
                    None => {
                        self.rejected_count += 1;
                        let err = AccountError::NoAvailableAccount(format!(
                            "No available accounts for engine '{}'",
                            engine
                        ));
                        ledger.record_event(
                            AuditEventType::CommandBlocked,
                            ActionSeverity::Destructive,
                            format!("Account pool empty for engine '{}'", engine),
                        );
                        return Err(err);
                    }
                }
            }
        }
    }

    pub fn select_batch(
        &mut self,
        engines: &[AccountEngineKind],
    ) -> Vec<Result<AccountProfile, AccountError>> {
        let mut results = Vec::with_capacity(engines.len());
        for engine in engines {
            results.push(self.select_account(engine));
        }
        results
    }
}
