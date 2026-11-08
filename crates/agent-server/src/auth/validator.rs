use axum::extract::Request;
use axum::http::HeaderValue;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use super::extractors::{extract_bearer_token, extract_ws_token, token_matches};
use super::fault::AuthFaultPolicy;
use super::types::AuthError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthFallbackStrategy {
    Reject,
    SecondaryToken(Arc<String>),
}

#[derive(Debug, Clone)]
pub struct ResilientAuthValidator {
    token: Arc<String>,
    fallback_token: Option<Arc<String>>,
    allowed_origins: Vec<HeaderValue>,
    fault_policy: AuthFaultPolicy,
    max_retries: usize,
    backoff_ms: u64,
    validated_count: usize,
    recovered_count: usize,
    rejected_count: usize,
    fallback_count: usize,
    max_capacity: Option<usize>,
    history: VecDeque<String>,
}

impl ResilientAuthValidator {
    pub fn new(
        token: Arc<String>,
        allowed_origins: Vec<HeaderValue>,
        fault_policy: AuthFaultPolicy,
        max_retries: usize,
    ) -> Self {
        Self {
            token,
            fallback_token: None,
            allowed_origins,
            fault_policy,
            max_retries,
            backoff_ms: 0,
            validated_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            fallback_count: 0,
            max_capacity: None,
            history: VecDeque::new(),
        }
    }

    pub fn with_fallback_token(mut self, fallback: Arc<String>) -> Self {
        self.fallback_token = Some(fallback);
        self
    }

    pub fn with_fallback_strategy(mut self, strategy: AuthFallbackStrategy) -> Self {
        match strategy {
            AuthFallbackStrategy::Reject => self.fallback_token = None,
            AuthFallbackStrategy::SecondaryToken(token) => self.fallback_token = Some(token),
        }
        self
    }

    pub fn with_backoff_ms(mut self, ms: u64) -> Self {
        self.backoff_ms = ms;
        self
    }

    pub fn with_max_capacity(mut self, max: usize) -> Self {
        self.max_capacity = Some(max);
        self
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let token_bytes = self.token.len() + self.fallback_token.as_ref().map_or(0, |t| t.len());
        let hist_bytes: usize = self
            .history
            .iter()
            .map(|s| s.len() + std::mem::size_of::<String>())
            .sum();
        base + token_bytes + hist_bytes
    }

    pub fn reset_counts(&mut self) {
        self.validated_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
        self.fallback_count = 0;
    }

    fn record_history(&mut self, candidate: &str) {
        if let Some(cap) = self.max_capacity {
            if self.history.len() >= cap {
                self.history.pop_front();
            }
            self.history.push_back(candidate.to_string());
        }
    }

    pub fn validate_batch_tokens(&mut self, tokens: &[&str]) -> Vec<Result<bool, AuthError>> {
        let mut results = Vec::with_capacity(tokens.len());
        for token in tokens {
            results.push(self.validate_token_resilient(token));
        }
        results
    }

    pub fn increment_validated(&mut self) {
        self.validated_count = self.validated_count.saturating_add(1);
    }

    pub fn increment_rejected(&mut self) {
        self.rejected_count = self.rejected_count.saturating_add(1);
    }

    pub fn validated_count(&self) -> usize {
        self.validated_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn fallback_count(&self) -> usize {
        self.fallback_count
    }

    pub fn fault_policy(&self) -> &AuthFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut AuthFaultPolicy {
        &mut self.fault_policy
    }

    pub fn is_retryable_error(&self, err: &AuthError) -> bool {
        err.is_retryable()
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

    pub fn validate_request_resilient(&mut self, request: &Request) -> Result<bool, AuthError> {
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

            if let Some(origin) = request.headers().get("origin") {
                if !self.allowed_origins.contains(origin) {
                    self.rejected_count += 1;
                    return Err(AuthError::Forbidden);
                }
            }

            let bearer = extract_bearer_token(request)?;
            let ws_token = extract_ws_token(request)?;

            let token = bearer.or(ws_token);
            match token {
                Some(t) if token_matches(t, &self.token) => {
                    self.validated_count += 1;
                    return Ok(true);
                }
                Some(t)
                    if self
                        .fallback_token
                        .as_ref()
                        .is_some_and(|f| token_matches(t, f)) =>
                {
                    self.validated_count += 1;
                    self.fallback_count += 1;
                    return Ok(true);
                }
                _ => {
                    self.rejected_count += 1;
                    return Err(AuthError::Unauthorized);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthFaultPolicy, ResilientAuthValidator};
    use crate::auth::{AuthError, AuthFault};
    use axum::body::Body;
    use axum::http::{HeaderValue, Request};
    use std::sync::Arc;

    fn validator() -> ResilientAuthValidator {
        ResilientAuthValidator::new(
            Arc::new("primary-token".into()),
            vec![],
            AuthFaultPolicy::none(),
            2,
        )
    }

    #[test]
    fn max_capacity_returns_configured_history_limit() {
        assert_eq!(validator().max_capacity(), None);
        assert_eq!(validator().with_max_capacity(2).max_capacity(), Some(2));
    }

    #[test]
    fn increment_validated_counts_calls_without_changing_other_counters() {
        let mut validator = validator();

        validator.increment_validated();
        validator.increment_validated();

        assert_eq!(validator.validated_count(), 2);
        assert_eq!(validator.rejected_count(), 0);
        assert_eq!(validator.recovered_count(), 0);
        assert_eq!(validator.fallback_count(), 0);
    }

    #[test]
    fn increment_validated_saturates_at_usize_max() {
        let mut validator = validator();
        validator.validated_count = usize::MAX;

        validator.increment_validated();

        assert_eq!(validator.validated_count(), usize::MAX);
    }

    #[test]
    fn increment_rejected_counts_calls_without_changing_other_counters() {
        let mut validator = validator();

        validator.increment_rejected();
        validator.increment_rejected();

        assert_eq!(validator.rejected_count(), 2);
        assert_eq!(validator.validated_count(), 0);
        assert_eq!(validator.recovered_count(), 0);
        assert_eq!(validator.fallback_count(), 0);
    }

    #[test]
    fn increment_rejected_saturates_at_usize_max() {
        let mut validator = validator();
        validator.rejected_count = usize::MAX;

        validator.increment_rejected();

        assert_eq!(validator.rejected_count(), usize::MAX);
    }

    #[test]
    fn reset_counts_clears_validation_and_rejection_counters() {
        let mut validator = validator().with_fallback_token(Arc::new("fallback-token".into()));
        assert_eq!(
            validator.validate_token_resilient("primary-token"),
            Ok(true)
        );
        assert_eq!(
            validator.validate_token_resilient("fallback-token"),
            Ok(true)
        );
        assert_eq!(validator.validate_token_resilient("wrong-token"), Ok(false));
        assert_eq!(validator.validated_count(), 2);
        assert_eq!(validator.fallback_count(), 1);
        assert_eq!(validator.rejected_count(), 1);

        validator.reset_counts();

        assert_eq!(validator.validated_count(), 0);
        assert_eq!(validator.recovered_count(), 0);
        assert_eq!(validator.fallback_count(), 0);
        assert_eq!(validator.rejected_count(), 0);
    }

    #[test]
    fn fault_policy_mut_changes_fault_injection_behavior() {
        let mut validator = validator();
        *validator.fault_policy_mut() = AuthFaultPolicy::always(AuthFault::TokenDecodingFailure);

        assert_eq!(
            validator.validate_token_resilient("primary-token"),
            Err(AuthError::TokenDecodingFailure(
                "Failed to decode token from request".into()
            ))
        );
    }

    #[test]
    fn is_retryable_error_classifies_transient_and_client_errors() {
        let validator = validator();
        assert!(validator.is_retryable_error(&AuthError::SocketTimeout));
        assert!(!validator.is_retryable_error(&AuthError::Unauthorized));
    }

    #[test]
    fn validate_request_resilient_accepts_authorized_bearer_token() {
        let mut validator = validator();
        let request = Request::builder()
            .header(
                "authorization",
                HeaderValue::from_static("Bearer primary-token"),
            )
            .body(Body::empty())
            .expect("request builds");

        assert_eq!(validator.validate_request_resilient(&request), Ok(true));
        assert_eq!(validator.validated_count(), 1);
    }
}
