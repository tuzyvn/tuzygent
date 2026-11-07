use axum::{body::Body, http::Request, http::StatusCode};
use tower::ServiceExt;

use super::common::app;

#[tokio::test]
async fn health_is_public_and_every_api_route_requires_auth(
) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        app()?
            .oneshot(Request::builder().uri("/health").body(Body::empty())?)
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app()?
            .oneshot(Request::builder().uri("/dashboard").body(Body::empty())?)
            .await?
            .status(),
        StatusCode::OK
    );
    for (method, path) in [
        ("GET", "/api/v1/engines"),
        ("GET", "/api/v1/agents"),
        ("POST", "/api/v1/agents"),
        ("POST", "/api/v1/tasks/run"),
        ("GET", "/api/v1/ws"),
        ("GET", "/v1/models"),
        ("POST", "/v1/chat/completions"),
        ("GET", "/api/v1/auth/accounts"),
    ] {
        let response = app()?
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn token_and_origin_are_checked_independently() -> Result<(), Box<dyn std::error::Error>> {
    for (token, origin, expected) in [
        ("test-secret", None, StatusCode::OK),
        ("wrong", None, StatusCode::UNAUTHORIZED),
        (
            "test-secret",
            Some("https://evil.example"),
            StatusCode::FORBIDDEN,
        ),
        (
            "test-secret",
            Some("https://dashboard.example"),
            StatusCode::OK,
        ),
    ] {
        let mut request = Request::builder()
            .uri("/api/v1/agents")
            .header("authorization", format!("Bearer {token}"));
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        assert_eq!(
            app()?.oneshot(request.body(Body::empty())?).await?.status(),
            expected
        );
    }
    Ok(())
}

#[tokio::test]
async fn browser_preflight_is_allowed_only_for_configured_origins(
) -> Result<(), Box<dyn std::error::Error>> {
    for origin in ["https://dashboard.example", "https://evil.example"] {
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/api/v1/tasks")
            .header("origin", origin)
            .header("access-control-request-method", "POST")
            .header(
                "access-control-request-headers",
                "authorization,content-type,idempotency-key",
            )
            .body(Body::empty())?;
        let response = app()?.oneshot(request).await?;
        assert_eq!(
            response
                .headers()
                .contains_key("access-control-allow-origin"),
            origin == "https://dashboard.example"
        );
        if origin == "https://dashboard.example" {
            assert!(response
                .headers()
                .get("access-control-allow-headers")
                .unwrap()
                .to_str()?
                .contains("idempotency-key"));
        }
    }
    Ok(())
}

#[tokio::test]
async fn websocket_token_does_not_authenticate_rest() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()?
        .oneshot(
            Request::builder()
                .uri("/api/v1/agents")
                .header("sec-websocket-protocol", "tuzygent, test-secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app()?
        .oneshot(
            Request::builder()
                .uri("/api/v1/ws")
                .header("sec-websocket-protocol", "tuzygent, test-secret")
                .header("origin", "https://evil.example")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}
