use async_trait::async_trait;
use std::sync::atomic::Ordering;
use std::time::Duration;

use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use crate::cli::fault::{is_retryable_engine_error, CliFault};
use crate::cli::fault::CliFaultPolicy;
use crate::cli::resilience::ResilientCliEngine;
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

impl<E: ComputeEngine> ResilientCliEngine<E> {
    async fn execute_with_optional_ledger(
        &self,
        input: &EngineInput,
        mut ledger: Option<&mut AuditLedger>,
    ) -> Result<EngineOutput, EngineError> {
        let mut attempt = 0;
        loop {
            attempt += 1;

            let fault = {
                let mut policy = self.fault_policy.lock().map_err(|e| {
                    EngineError::ProcessFailed(format!("Fault policy lock poisoned: {}", e))
                })?;
                policy.should_inject()
            };

            if let Some(fault) = fault {
                match fault {
                    CliFault::SocketTimeout
                    | CliFault::ProcessTimeout
                    | CliFault::BrokenPipe
                    | CliFault::TransientIo
                    | CliFault::ProcessFailed => {
                        if attempt <= self.max_retries {
                            self.recovered_count.fetch_add(1, Ordering::SeqCst);
                            if let Some(ref mut l) = ledger {
                                l.record_event(
                            AuditEventType::CommandAllowed,
                            ActionSeverity::Routine,
                            format!(
                                "CLI transient fault {:?} intercepted, retrying attempt {}/{} for engine '{}'",
                                fault, attempt, self.max_retries, self.engine_kind()
                            ),
                        );
                            }
                            if let Some(ref policy) = self.backoff_policy {
                                policy.sleep_for_attempt(attempt as u32).await;
                            } else if self.retry_delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(self.retry_delay_ms))
                                    .await;
                            }
                            continue;
                        } else {
                            self.rejected_count.fetch_add(1, Ordering::SeqCst);
                            let err = fault.to_engine_error();
                            if let Some(ref mut l) = ledger {
                                l.record_event(
                                    AuditEventType::CommandException,
                                    ActionSeverity::HighImpact,
                                    format!(
                                        "CLI max retries exceeded for engine '{}': {}",
                                        self.engine_kind(),
                                        err
                                    ),
                                );
                            }
                            return Err(err);
                        }
                    }
                    CliFault::CorruptedOutput | CliFault::CliNotFound => {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        let err = fault.to_engine_error();
                        if let Some(ref mut l) = ledger {
                            l.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "Fatal CLI fault {:?} rejected for engine '{}': {}",
                                    fault,
                                    self.engine_kind(),
                                    err
                                ),
                            );
                        }
                        return Err(err);
                    }
                }
            }

            match self.inner.execute(input).await {
                Ok(output) => {
                    self.resolved_count.fetch_add(1, Ordering::SeqCst);
                    self.cache_response(&input.prompt_or_command, &output);
                    if let Some(ref mut l) = ledger {
                        l.record_event(
                            AuditEventType::CommandAllowed,
                            ActionSeverity::Routine,
                            format!(
                                "CLI execution succeeded with engine '{}'",
                                self.engine_kind()
                            ),
                        );
                    }
                    return Ok(output);
                }
                Err(err) => {
                    if is_retryable_engine_error(&err) && attempt <= self.max_retries {
                        self.recovered_count.fetch_add(1, Ordering::SeqCst);
                        if let Some(ref mut l) = ledger {
                            l.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!(
                                    "CLI error {:?} intercepted, retrying attempt {}/{} for engine '{}'",
                                    err, attempt, self.max_retries, self.engine_kind()
                                ),
                            );
                        }
                        if let Some(ref policy) = self.backoff_policy {
                            policy.sleep_for_attempt(attempt as u32).await;
                        } else if self.retry_delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                        }
                        continue;
                    } else {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        if let Some(ref mut l) = ledger {
                            l.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "CLI execution failed for engine '{}': {}",
                                    self.engine_kind(),
                                    err
                                ),
                            );
                        }
                        return Err(err);
                    }
                }
            }
        }
    }

    pub async fn execute_batch(
        &self,
        inputs: &[EngineInput],
    ) -> Vec<Result<EngineOutput, EngineError>> {
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results.push(self.execute(input).await);
        }
        results
    }

    pub async fn execute_and_record(
        &self,
        ledger: &mut AuditLedger,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        self.execute_with_optional_ledger(input, Some(ledger)).await
    }
}

#[async_trait]
impl<E: ComputeEngine> ComputeEngine for ResilientCliEngine<E> {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        self.execute_with_optional_ledger(input, None).await
    }

    fn engine_kind(&self) -> EngineKind {
        self.inner.engine_kind()
    }
}

pub trait ResilientCliWrapper: ComputeEngine + Sized {
    fn with_resilience(self, max_retries: usize) -> ResilientCliEngine<Self> {
        ResilientCliEngine::new(self, CliFaultPolicy::none(), max_retries)
    }

    fn with_chaos(
        self,
        fault_policy: CliFaultPolicy,
        max_retries: usize,
    ) -> ResilientCliEngine<Self> {
        ResilientCliEngine::new(self, fault_policy, max_retries)
    }
}

impl<T: ComputeEngine + Sized> ResilientCliWrapper for T {}
