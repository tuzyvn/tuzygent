use axum::extract::Request;
use super::types::AuthError;

pub fn token_matches(provided: &str, expected: &str) -> bool {
    provided.len() == expected.len()
        && provided
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

pub fn extract_bearer_token(request: &Request) -> Result<Option<&str>, AuthError> {
    let header = match request.headers().get("authorization") {
        Some(h) => h,
        None => return Ok(None),
    };

    let header_str = header.to_str().map_err(|e| {
        AuthError::TokenDecodingFailure(format!("Authorization header contains non-ASCII bytes: {e}"))
    })?;

    if let Some(token) = header_str.strip_prefix("Bearer ") {
        if token.is_empty() || token.starts_with(' ') || token.ends_with(' ') {
            return Err(AuthError::TokenDecodingFailure(
                "Bearer token is empty or contains malformed whitespace".to_string(),
            ));
        }
        Ok(Some(token))
    } else {
        Err(AuthError::MalformedHeader(
            "Authorization header missing 'Bearer ' prefix".to_string(),
        ))
    }
}

pub fn extract_ws_token(request: &Request) -> Result<Option<&str>, AuthError> {
    if request.uri().path() != "/api/v1/ws" {
        return Ok(None);
    }
    let header = match request.headers().get("sec-websocket-protocol") {
        Some(h) => h,
        None => return Ok(None),
    };

    let header_str = header.to_str().map_err(|e| {
        AuthError::TokenDecodingFailure(format!(
            "Sec-WebSocket-Protocol contains non-ASCII bytes: {e}"
        ))
    })?;

    let mut parts = header_str.split(',').map(str::trim);
    match (parts.next(), parts.next(), parts.next()) {
        (Some("tuzygent"), Some(token), None) if !token.is_empty() => Ok(Some(token)),
        _ => Err(AuthError::MalformedHeader(
            "Sec-WebSocket-Protocol must match 'tuzygent, <token>'".to_string(),
        )),
    }
}
