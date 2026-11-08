use std::time::Duration;

use axum::extract::Request;

use super::super::extractors::{extract_bearer_token, extract_ws_token, token_matches};
use super::ResilientAuthValidator;
use crate::auth::types::AuthError;

impl ResilientAuthValidator {
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
