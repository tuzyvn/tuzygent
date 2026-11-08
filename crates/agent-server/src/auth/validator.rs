use axum::http::HeaderValue;
use std::collections::VecDeque;
use std::sync::Arc;

use super::fault::AuthFaultPolicy;

mod metrics;
mod request_validation;
mod token_validation;

#[cfg(test)]
mod tests;

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
}
