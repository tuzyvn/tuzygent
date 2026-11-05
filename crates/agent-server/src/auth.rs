use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthState {
    pub token: Arc<String>,
    pub allowed_origins: Vec<HeaderValue>,
}

pub fn token_matches(provided: &str, expected: &str) -> bool {
    provided.len() == expected.len()
        && provided
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

pub async fn require_auth(
    State(auth): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    // CORS middleware does not validate WebSocket origins.
    if let Some(origin) = request.headers().get("origin") {
        if !auth.allowed_origins.contains(origin) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    // Browser WebSocket clients cannot set Authorization; pass ["tuzygent", token].
    let ws_token = if request.uri().path() == "/api/v1/ws" {
        request
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| {
                let mut parts = h.split(',').map(str::trim);
                match (parts.next(), parts.next(), parts.next()) {
                    (Some("tuzygent"), Some(token), None) => Some(token),
                    _ => None,
                }
            })
    } else {
        None
    };
    if !bearer
        .or(ws_token)
        .is_some_and(|token| token_matches(token, &auth.token))
    {
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
