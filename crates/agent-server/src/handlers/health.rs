use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: "0.1.0",
        timestamp: chrono::Utc::now(),
    })
}
