use std::sync::Arc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};

use super::types::{AccountEngineKind, AccountProfile};
use super::vault::AccountVault;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum AccountError {
    #[error("Socket timeout communicating with account service")]
    SocketTimeout,
    #[error("Connection reset by peer")]
    ConnectionReset,
    #[error("Rate limit or quota exceeded: {0}")]
    RateLimitExceeded(String),
    #[error("Transient I/O failure during account storage operation: {0}")]
    TransientIo(String),
    #[error("Corrupted account metadata detected: {0}")]
    CorruptedMetadata(String),
    #[error("Account service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Invalid account configuration: {0}")]
    InvalidAccount(String),
    #[error("No available accounts for engine kind: {0}")]
    NoAvailableAccount(String),
    #[error("Max retry attempts ({0}) exceeded during account recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountFault {
    SocketTimeout,
    ConnectionReset,
    RateLimitExceeded,
    TransientIo,
    CorruptedMetadata,
    ServiceUnavailable,
}

#[derive(Debug, Clone, Default)]
pub struct AccountFaultPolicy {
    fault: Option<AccountFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl AccountFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: AccountFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: AccountFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<AccountFault> {
        let fault = self.fault.as_ref()?;

        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);

        let inject = match self.fail_interval {
            Some(interval) => (self.invocation_count - 1).is_multiple_of(interval),
            None => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }
}

#[derive(Clone)]
pub struct ResilientAccountManager {
    vault: Arc<AccountVault>,
    fault_policy: AccountFaultPolicy,
    max_retries: usize,
    resolved_count: usize,
    recovered_count: usize,
    rejected_count: usize,
    cooldown_count: usize,
}

impl ResilientAccountManager {
    pub fn new(vault: Arc<AccountVault>, fault_policy: AccountFaultPolicy, max_retries: usize) -> Self {
        Self {
            vault,
            fault_policy,
            max_retries,
            resolved_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            cooldown_count: 0,
        }
    }

    pub fn with_vault(vault: Arc<AccountVault>) -> Self {
        Self::new(vault, AccountFaultPolicy::none(), 3)
    }

    pub fn with_fault_policy(vault: Arc<AccountVault>, fault_policy: AccountFaultPolicy) -> Self {
        Self::new(vault, fault_policy, 3)
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

    pub fn select_account(&mut self, engine: &AccountEngineKind) -> Result<AccountProfile, AccountError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    AccountFault::RateLimitExceeded => {
                        if let Some(acc) = self.vault.select_next_account(engine) {
                            self.vault.mark_cooldown(engine, &acc.id, 60, "RateLimitExceeded (Chaos Injected)");
                            self.cooldown_count += 1;
                            self.recovered_count += 1;
                            if attempt <= self.max_retries {
                                continue;
                            } else {
                                return Err(AccountError::RateLimitExceeded(
                                    "Rate limit exceeded across account pool".to_string(),
                                ));
                            }
                        } else {
                            return Err(AccountError::NoAvailableAccount(format!(
                                "All accounts for engine '{}' are exhausted or in cooldown",
                                engine
                            )));
                        }
                    }
                    AccountFault::SocketTimeout
                    | AccountFault::ConnectionReset
                    | AccountFault::TransientIo
                    | AccountFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            return match fault {
                                AccountFault::SocketTimeout => Err(AccountError::SocketTimeout),
                                AccountFault::ConnectionReset => Err(AccountError::ConnectionReset),
                                AccountFault::TransientIo => Err(AccountError::TransientIo(
                                    "Simulated transient I/O failure during account storage operation".to_string(),
                                )),
                                AccountFault::ServiceUnavailable => Err(AccountError::ServiceUnavailable(
                                    "Account service temporarily unavailable".to_string(),
                                )),
                                _ => unreachable!(),
                            };
                        }
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
                            self.resolved_count += 1;
                            return Ok(profile);
                        }
                        Err(err) => {
                            self.rejected_count += 1;
                            return Err(err);
                        }
                    },
                    None => {
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
                            self.vault.mark_cooldown(engine, &acc.id, 60, "RateLimitExceeded (Chaos Injected)");
                            self.cooldown_count += 1;
                            self.recovered_count += 1;
                            ledger.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!("Account '{}' rate limited; cooldown triggered and rotated", acc.id),
                            );
                            if attempt <= self.max_retries {
                                continue;
                            } else {
                                let err = AccountError::RateLimitExceeded(
                                    "Rate limit exceeded across account pool".to_string(),
                                );
                                ledger.record_event(
                                    AuditEventType::CommandException,
                                    ActionSeverity::HighImpact,
                                    format!("Account selection exception for engine '{}': {}", engine, err),
                                );
                                return Err(err);
                            }
                        } else {
                            let err = AccountError::NoAvailableAccount(format!(
                                "All accounts for engine '{}' are exhausted or in cooldown",
                                engine
                            ));
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!("Account selection exception for engine '{}': {}", engine, err),
                            );
                            return Err(err);
                        }
                    }
                    AccountFault::SocketTimeout
                    | AccountFault::ConnectionReset
                    | AccountFault::TransientIo
                    | AccountFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            let err = match fault {
                                AccountFault::SocketTimeout => AccountError::SocketTimeout,
                                AccountFault::ConnectionReset => AccountError::ConnectionReset,
                                AccountFault::TransientIo => AccountError::TransientIo(
                                    "Simulated transient I/O failure during account storage operation".to_string(),
                                ),
                                AccountFault::ServiceUnavailable => AccountError::ServiceUnavailable(
                                    "Account service temporarily unavailable".to_string(),
                                ),
                                _ => unreachable!(),
                            };
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!("Account selection exception for engine '{}': {}", engine, err),
                            );
                            return Err(err);
                        }
                    }
                    AccountFault::CorruptedMetadata => {
                        let err = AccountError::CorruptedMetadata(
                            "Account profile metadata corruption detected".to_string(),
                        );
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!("Account selection exception for engine '{}': {}", engine, err),
                        );
                        return Err(err);
                    }
                }
            } else {
                match self.vault.select_next_account(engine) {
                    Some(profile) => match profile.validate() {
                        Ok(()) => {
                            self.resolved_count += 1;
                            ledger.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!("Account '{}' resolved for engine '{}'", profile.id, engine),
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
}
