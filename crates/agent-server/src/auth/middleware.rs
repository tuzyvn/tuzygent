use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::extractors::{extract_bearer_token, extract_ws_token, token_matches};
use super::state::AuthState;

pub async fn require_auth(
    State(auth): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    // Fault injection check for chaos simulation
    if let Ok(mut policy) = auth.fault_policy.lock() {
        if let Some(fault) = policy.should_inject() {
            let err = fault.to_auth_error();
            return err.to_status_code().into_response();
        }
    }

    // CORS middleware does not validate WebSocket origins.
    if let Some(origin) = request.headers().get("origin") {
        if !auth.allowed_origins.contains(origin) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    let bearer = match extract_bearer_token(&request) {
        Ok(t) => t,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };

    let ws_token = match extract_ws_token(&request) {
        Ok(t) => t,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };

    let candidate = bearer.or(ws_token);
    let is_valid = match candidate {
        Some(token) => {
            if let Some(ref cache) = auth.cache {
                if let Some(cached_valid) = cache.get(token) {
                    cached_valid
                } else {
                    let valid = token_matches(token, &auth.token)
                        || auth
                            .fallback_token
                            .as_ref()
                            .is_some_and(|fallback| token_matches(token, fallback));
                    cache.insert(token, valid);
                    valid
                }
            } else {
                token_matches(token, &auth.token)
                    || auth
                        .fallback_token
                        .as_ref()
                        .is_some_and(|fallback| token_matches(token, fallback))
            }
        }
        None => false,
    };

    if !is_valid {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_matches_constant_time() {
        assert!(token_matches("secret123", "secret123"));
        assert!(!token_matches("secret123", "secret124"));
        assert!(!token_matches("secret", "secret123"));
        assert!(!token_matches("secret123", "secret"));
        assert!(token_matches("", ""));
    }
}
