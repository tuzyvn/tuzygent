use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

pub async fn metrics_handler(State(state): State<AppState>) -> Response {
    match state.metrics.render_prometheus_text() {
        Ok(text) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            )],
            text,
        )
            .into_response(),
        Err(err) => (
            err.to_status_code(),
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            )],
            format!("# Error exporting metrics: {}\n", err),
        )
            .into_response(),
    }
}
