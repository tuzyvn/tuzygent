
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

#[test]
fn validate_batch_tokens_preserves_order_and_updates_each_result_counter() {
    let mut validator = validator().with_fallback_token(Arc::new("fallback-token".into()));

    let results =
        validator.validate_batch_tokens(&["primary-token", "wrong-token", "fallback-token"]);

    assert_eq!(results, vec![Ok(true), Ok(false), Ok(true)]);
    assert_eq!(validator.validated_count(), 2);
    assert_eq!(validator.rejected_count(), 1);
    assert_eq!(validator.fallback_count(), 1);
}

#[test]
fn validate_request_resilient_rejects_disallowed_origin_before_token_validation() {
    let mut validator = ResilientAuthValidator::new(
        Arc::new("primary-token".into()),
        vec![HeaderValue::from_static("https://trusted.example")],
        AuthFaultPolicy::none(),
        2,
    );
    let request = Request::builder()
        .header("origin", "https://untrusted.example")
        .header(
            "authorization",
            HeaderValue::from_static("Bearer primary-token"),
        )
        .body(Body::empty())
        .expect("request builds");

    assert_eq!(
        validator.validate_request_resilient(&request),
        Err(AuthError::Forbidden)
    );
    assert_eq!(validator.validated_count(), 0);
    assert_eq!(validator.rejected_count(), 1);
}
