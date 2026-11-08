use std::time::Duration;

use super::super::extractors::token_matches;
use super::ResilientAuthValidator;
use crate::auth::types::AuthError;

impl ResilientAuthValidator {
    pub fn validate_batch_tokens(&mut self, tokens: &[&str]) -> Vec<Result<bool, AuthError>> {
        let mut results = Vec::with_capacity(tokens.len());
        for token in tokens {
            results.push(self.validate_token_resilient(token));
        }
        results
    }

    pub fn validate_token_resilient(&mut self, provided: &str) -> Result<bool, AuthError> {
        let mut retries = 0;
        loop {
            if let Some(fault) = self.fault_policy.should_inject() {
                let err = fault.to_auth_error();
                if self.is_retryable_error(&err) && retries < self.max_retries {
                    retries += 1;
                    self.recovered_count += 1;
                    if self.backoff_ms > 0 {
                        std::thread::sleep(Duration::from_millis(self.backoff_ms * retries as u64));
                    }
                    continue;
                }
                self.rejected_count += 1;
                if retries >= self.max_retries && self.is_retryable_error(&err) {
                    return Err(AuthError::MaxRetriesExceeded(self.max_retries));
                }
                return Err(err);
            }

            self.record_history(provided);

            if token_matches(provided, &self.token) {
                self.validated_count += 1;
                return Ok(true);
            }

            if let Some(fallback) = &self.fallback_token {
                if token_matches(provided, fallback) {
                    self.validated_count += 1;
                    self.fallback_count += 1;
                    return Ok(true);
                }
            }

            self.rejected_count += 1;
            return Ok(false);
        }
    }
}
