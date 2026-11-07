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
        version: env!("CARGO_PKG_VERSION"),
        timestamp: chrono::Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::HealthResponse;
    use chrono::{TimeZone, Utc};
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct DeserializedHealthResponse {
        status: String,
        version: String,
        timestamp: chrono::DateTime<Utc>,
    }

    #[test]
    fn health_response_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        let timestamp = Utc.with_ymd_and_hms(2026, 11, 7, 12, 34, 56).unwrap();
        let response = HealthResponse {
            status: "ok",
            version: "1.2.3",
            timestamp,
        };

        let value = serde_json::to_value(&response)?;
        assert_eq!(
            value,
            serde_json::json!({
                "status": "ok",
                "version": "1.2.3",
                "timestamp": "2026-11-07T12:34:56Z"
            })
        );

        let deserialized: DeserializedHealthResponse = serde_json::from_value(value)?;
        assert_eq!(
            deserialized,
            DeserializedHealthResponse {
                status: "ok".to_owned(),
                version: "1.2.3".to_owned(),
                timestamp,
            }
        );
        Ok(())
    }
}
