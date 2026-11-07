use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};

use super::ResilientAccountManager;
use crate::accounts::fault::{AccountError, AccountFault};
use crate::accounts::types::{AccountEngineKind, AccountProfile};

impl ResilientAccountManager {
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
